#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
source "$REPO/tools/lib.sh"
require_bash 4.4 "$REPO/tools/release.sh" "$@"
shopt -s inherit_errexit
LIB_CRATE="ownpg-core"
BIN_CRATE="ownpg"
ROOT_MANIFEST="Cargo.toml"
CLI_MANIFEST="crates/ownpg/Cargo.toml"
LOCK_FILE="Cargo.lock"
CHANGELOG="CHANGELOG.md"
ATTRIBUTION="THIRD-PARTY.txt"
MCPB_MANIFEST="mcpb/manifest.json"
REGISTRY_MANIFEST="server.json"
MINISIGN_PUB="${OWNPG_MINISIGN_PUB:-minisign.pub}"
BRANCH="main"
PROTECTED_BRANCHES=(main master develop staging production)
REMOTE="origin"
PUBLIC_RELEASE_REPO="devops-infinity/ownpg-releases"
RELEASE_URL_BASE="https://github.com/devops-infinity/ownpg-releases/releases/tag"
CRATES_API="https://crates.io/api/v1/crates"
USER_AGENT="ownpg-release-script (+https://github.com/devops-infinity/ownpg-releases)"
MINISIGN_KEY="${OWNPG_MINISIGN_KEY:-$HOME/.minisign/minisign.key}"
PROPAGATE_TRIES=30
PROPAGATE_WAIT=10
PROPAGATE_JITTER=5
JQ_FLOOR="1.6"
CURL_FLOOR="7.52.0"

MODE="release"
VERSION=""
BUMP=""
CURRENT_VERSION=""
ASSUME_YES=0
STEP="startup"
STEP_NO=0
WORK_DIR=""
LOCK_DIR=""
MUTATED=0
IRREVERSIBLE=0
UPLOADED=0
RESUME_REQUESTED=0
LIB_PUBLISHED=0
BIN_PUBLISHED=0
PUSHED=0
CRATE_HTTP_STATUS=""
CRATE_BODY_FILE=""

usage() {
	cat <<'USAGE'
Usage: tools/release.sh --prepare --patch | --minor | --major | --version X.Y.Z
       tools/release.sh --version X.Y.Z [options]
       tools/release.sh --patch | --minor | --major --branch NAME [options]
       tools/release.sh --dry-run --patch
       tools/release.sh --resume --version X.Y.Z
       tools/release.sh --yank X.Y.Z | --unyank X.Y.Z

  --major          release the next major version, worked out from the current one
  --minor          release the next minor version
  --patch          release the next patch version
  --version X.Y.Z  release this exact version instead of a computed one
  --prepare        write the version bump and the changelog heading into the
                   working tree and stop; nothing is published, committed, or
                   pushed, so the bump reaches main through a pull request
  --dry-run        run every check and the bump, publish nothing, revert the bump
  --resume         finish a release that stopped part way: every step that
                   already happened (a crate on crates.io, the commit, the
                   tag, a GitHub release, the npm version) is checked and
                   skipped; needs --version X.Y.Z
  --yes            skip the typed confirmation, for unattended runs
  --yank X.Y.Z     yank that version from both crates, binary first
  --unyank X.Y.Z   put a yanked version back, library first
  --branch NAME    release from this branch instead of main, for a hotfix cut
                   from a release tag (the branch must exist on the remote)
  -h, --help       show this

Release order: ownpg-core is published before ownpg, because the binary
depends on the library. Nothing is committed, tagged, or pushed until both
crates are on crates.io. Prebuilt binaries are then built with dist for
whichever of the eight targets this machine supports, the checksum file is
signed with minisign, and a GitHub release is created and uploaded last, after
the tag is pushed, on this repository and again on the public
devops-infinity/ownpg-releases repository the installer scripts, the Homebrew
formula, the npm package, and cargo-binstall actually resolve against.

The script never commits to main, master, develop, staging, or production. A
release from one of them needs the bump already merged there: run --prepare on
a new branch, merge it through a pull request, then run --version X.Y.Z from
the base branch, and only the tag is pushed. From any other branch given with
--branch, the bump is committed and pushed to that branch.

Environment:
  OWNPG_MINISIGN_KEY        the minisign secret key (default ~/.minisign/minisign.key)
  OWNPG_MINISIGN_PUB        the minisign public key file name (default minisign.pub)
  OWNPG_CODESIGN_IDENTITY   Developer ID Application identity for the macOS binaries
  OWNPG_NOTARY_PROFILE      notarytool keychain profile; unset skips signing with a warning
  OWNPG_WINDOWS_SIGN_CERT   PKCS#12 certificate for the Windows binaries, signed with osslsigncode
  OWNPG_WINDOWS_SIGN_PASS   the password of that certificate; unset skips signing with a warning
  OWNPG_HOMEBREW_TAP        the tap repository (default devops-infinity/homebrew-tap)
  OWNPG_SKIP_TAP            set to 1 to leave the formula in target/distrib instead of pushing it
  OWNPG_SKIP_NPM            set to 1 to leave the npm package in target/distrib instead of publishing it
  OWNPG_ALLOW_PARTIAL       set to 1 to continue past a missing cross toolchain instead of stopping
  OWNPG_TEST_DSN            the database tools/verify.sh runs the live tests against; see that script
  OWNPG_SKIP_LIVE_TESTS     set to 1 to let tools/verify.sh skip the live tests when OWNPG_TEST_DSN is unset
USAGE
}

step() {
	STEP_NO=$((STEP_NO + 1))
	STEP="$1"
	say INFO "step $STEP_NO: $1"
}

on_error() {
	say FAILED "step '$STEP' failed with exit $1"
}

print_manual_binary_steps() {
	printf '  dist build --tag=v%s --artifacts=local --target=<triple> --no-local-paths --output-format=json   # once per target\n' "$VERSION"
	printf '  dist build --tag=v%s --artifacts=global --no-local-paths\n' "$VERSION"
	printf '  minisign -Sm target/distrib/sha256.sum -s %s\n' "$MINISIGN_KEY"
	printf '  gh release create v%s <the files under target/distrib, not the directories> --title "OwnPG %s" --notes-file <notes-file>\n' "$VERSION" "$VERSION"
	printf '  gh release create v%s <the same files> --repo %s --title "OwnPG %s" --notes-file <notes-file>\n' "$VERSION" "$PUBLIC_RELEASE_REPO" "$VERSION"
}

print_manual_finish() {
	say INFO "finish with: tools/release.sh --resume --version $VERSION"
	say INFO "or by hand with:"
	if ! branch_is_protected "$BRANCH"; then
		printf '  git add -- %s\n' "$(tracked_release_files | tr '\n' ' ')"
		printf '  git commit -m "chore: release %s"\n' "$VERSION"
	fi
	printf '  git tag %s v%s -m "OwnPG v%s"\n' "$(tag_flag)" "$VERSION" "$VERSION"
	if ! branch_is_protected "$BRANCH"; then
		printf '  git push %s %s\n' "$REMOTE" "$BRANCH"
	fi
	printf '  git push %s v%s\n' "$REMOTE" "$VERSION"
	print_manual_binary_steps
}

cleanup() {
	local status=$?
	if [[ $MUTATED -eq 1 ]]; then
		if [[ $IRREVERSIBLE -eq 0 ]]; then
			restore_tree
			say WARNING "the version bump was reverted; the tree is back on $CURRENT_VERSION and nothing was published"
		else
			say WARNING "a crate was already uploaded, so the bump was left in place"
			print_manual_finish
		fi
	elif [[ $PUSHED -eq 1 ]]; then
		say WARNING "the commit, tag, and push already succeeded; finish the binaries by hand with:"
		print_manual_binary_steps
	fi
	if [[ -n "$WORK_DIR" && -d "$WORK_DIR" ]]; then
		rm -f -- "$WORK_DIR/windows-sign.pass"
		if [[ $status -ne 0 ]]; then
			say INFO "full logs kept at: $WORK_DIR"
		else
			rm -rf -- "$WORK_DIR"
		fi
	fi
	if [[ -n "$LOCK_DIR" ]]; then
		release_pid_lock "$LOCK_DIR"
	fi
}

trap 'on_error $?' ERR
trap cleanup EXIT

log_path_for() {
	printf '%s/%s.log' "$WORK_DIR" "$(printf '%s' "$1" | tr -c 'A-Za-z0-9' '-')"
}

run() {
	local label="$1"
	shift
	STEP="$label"
	local log code=0
	log="$(log_path_for "$label")"
	"$@" >"$log" 2>&1 || code=$?
	if [[ $code -ne 0 ]]; then
		say INFO "last 40 lines of output:"
		tail -n 40 "$log" >&2
		say FAILED "$label (exit $code)"
		exit "$code"
	fi
	say SUCCESS "$label"
}

require_tools() {
	local tool
	for tool in "$@"; do
		command -v "$tool" >/dev/null 2>&1 || die "$tool is not on PATH"
	done
}

require_tool_version() {
	local tool="$1" reported="$2" floor="$3" hint="$4" found=""
	if [[ "$reported" =~ ([0-9]+\.[0-9]+(\.[0-9]+)?) ]]; then
		found="${BASH_REMATCH[1]}"
	fi
	[[ -n "$found" ]] || die "could not read a version number for $tool from: $reported"
	version_at_least "$found" "$floor" ||
		die "$tool $found is first on PATH, but this script needs $tool $floor or newer; $hint"
	say SUCCESS "$tool $found"
}

require_credentials() {
	if [[ -f "$HOME/.cargo/credentials.toml" ]]; then
		return 0
	fi
	if [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
		say INFO "using CARGO_REGISTRY_TOKEN from the environment"
		return 0
	fi
	die "no crates.io credentials: run 'cargo login' or set CARGO_REGISTRY_TOKEN"
}

require_npm_auth() {
	[[ "${OWNPG_SKIP_NPM:-0}" == "1" ]] && return 0
	require_tools npm
	npm whoami >/dev/null 2>&1 ||
		die "npm is not logged in; put a publish token that bypasses 2FA in ~/.npmrc, or set OWNPG_SKIP_NPM=1"
	say SUCCESS "npm is logged in"
}

require_gh_auth() {
	gh auth status >/dev/null 2>&1 || die "gh is not authenticated: run 'gh auth login'"
}

branch_is_protected() {
	local name
	for name in "${PROTECTED_BRANCHES[@]}"; do
		[[ "$1" == "$name" ]] && return 0
	done
	return 1
}

require_prepared_release() {
	local fix="land the bump through a pull request first (tools/release.sh --prepare --version $VERSION on a new branch), then rerun this from $BRANCH"
	[[ "$VERSION" == "$CURRENT_VERSION" ]] ||
		die "tools/release.sh never commits to $BRANCH, which is on $CURRENT_VERSION, not $VERSION; $fix"
	[[ -f "$CHANGELOG" ]] || return 0
	grep -qF "## [$VERSION]" "$CHANGELOG" ||
		die "tools/release.sh never commits to $BRANCH, and $CHANGELOG has no [$VERSION] section; $fix"
	grep -qF "[$VERSION]: " "$CHANGELOG" ||
		die "tools/release.sh never commits to $BRANCH, and $CHANGELOG has no [$VERSION] link; $fix"
}

refuse_commit_on_protected_branch() {
	branch_is_protected "$BRANCH" || return 0
	local -a files=()
	local changed
	mapfile -t files < <(tracked_release_files)
	changed="$(git status --porcelain -- "${files[@]}" | cut -c4- | tr '\n' ' ')"
	[[ -n "${changed// /}" ]] || return 0
	restore_tree
	die "tools/release.sh never commits to $BRANCH, but this release would change: ${changed% }; land that through a pull request (tools/release.sh --prepare --version $VERSION on a new branch), then rerun"
}

require_minisign_key() {
	[[ -f "$MINISIGN_KEY" ]] || die "no minisign secret key at $MINISIGN_KEY; run 'minisign -G' once, or point OWNPG_MINISIGN_KEY at the key"
}

require_llvm_tools() {
	local sysroot
	sysroot="$(rustc --print sysroot)" || die "rustc could not report its sysroot"
	[[ -x "$sysroot/lib/rustlib/$(host_triple)/bin/llvm-ar" ]] ||
		die "the llvm-tools component is missing, and cargo-xwin needs its llvm-lib for the Windows ARM64 build; run: rustup component add llvm-tools"
}

check_dist_version() {
	local declared installed
	declared="$(awk '
		/^\[workspace\.metadata\.dist\]/ { in_section = 1; next }
		/^\[/ { in_section = 0 }
		in_section && /^cargo-dist-version[[:space:]]*=/ {
			if (match($0, /"[^"]*"/)) {
				print substr($0, RSTART + 1, RLENGTH - 2)
				exit
			}
		}
	' "$ROOT_MANIFEST")"
	[[ -n "$declared" ]] || die "no cargo-dist-version in [workspace.metadata.dist] of $ROOT_MANIFEST"
	installed="$(dist --version | awk '{print $2}')"
	[[ "$installed" == "$declared" ]] ||
		die "dist $installed is on PATH but $ROOT_MANIFEST declares cargo-dist-version = \"$declared\"; install $declared or update the manifest"
}

valid_version() {
	[[ "$1" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
}

next_version() {
	local current="$1" kind="$2" major minor patch
	IFS=. read -r major minor patch <<<"$current"
	case "$kind" in
	major) printf '%d.0.0\n' "$((major + 1))" ;;
	minor) printf '%d.%d.0\n' "$major" "$((minor + 1))" ;;
	patch) printf '%d.%d.%d\n' "$major" "$minor" "$((patch + 1))" ;;
	*) return 1 ;;
	esac
}

read_current_version() {
	awk '
		/^\[/ { section = $0 }
		section == "[workspace.package]" && /^version[[:space:]]*=/ {
			if (match($0, /"[^"]*"/)) {
				print substr($0, RSTART + 1, RLENGTH - 2)
				exit
			}
		}
	' "$ROOT_MANIFEST"
}

version_greater() {
	local -a new old
	IFS=. read -r -a new <<<"$1"
	IFS=. read -r -a old <<<"$2"
	local i
	for i in 0 1 2; do
		if ((10#${new[i]} > 10#${old[i]})); then
			return 0
		fi
		if ((10#${new[i]} < 10#${old[i]})); then
			return 1
		fi
	done
	return 1
}

fetch_crate() {
	local name="$1"
	CRATE_BODY_FILE="$WORK_DIR/crate-$name.json"
	CRATE_HTTP_STATUS="$(curl -sS -A "$USER_AGENT" --connect-timeout 10 --max-time 30 \
		--retry 3 --retry-delay 2 --retry-max-time 60 -o "$CRATE_BODY_FILE" \
		-w '%{http_code}' "$CRATES_API/$name" 2>/dev/null)" || CRATE_HTTP_STATUS="000"
}

crate_has_version() {
	jq -e --arg want "$1" '[.versions[].num] | index($want) != null' "$CRATE_BODY_FILE" >/dev/null 2>&1
}

published_already() {
	local name="$1"
	fetch_crate "$name"
	case "$CRATE_HTTP_STATUS" in
	404)
		say INFO "$name is not on crates.io yet; this would be its first release"
		return 1
		;;
	200)
		crate_has_version "$VERSION"
		;;
	000)
		die "could not reach crates.io for $name (network error); check the connection and try again"
		;;
	*)
		die "crates.io answered $CRATE_HTTP_STATUS for $name"
		;;
	esac
}

check_registry_state() {
	if published_already "$LIB_CRATE"; then LIB_PUBLISHED=1; fi
	if published_already "$BIN_CRATE"; then BIN_PUBLISHED=1; fi

	if [[ $BIN_PUBLISHED -eq 1 && $LIB_PUBLISHED -eq 0 ]]; then
		die "$BIN_CRATE $VERSION is on crates.io but $LIB_CRATE $VERSION is not; sort that out by hand"
	fi
	if [[ $LIB_PUBLISHED -eq 1 ]]; then
		[[ $RESUME_REQUESTED -eq 1 ]] ||
			die "$VERSION is already on crates.io for $LIB_CRATE$([[ $BIN_PUBLISHED -eq 1 ]] && printf ' and %s' "$BIN_CRATE"); a published version cannot be replaced, so finish it with: tools/release.sh --resume --version $VERSION"
		IRREVERSIBLE=1
		say WARNING "this run resumes a part-finished $VERSION release; steps that already happened are skipped"
		return 0
	fi
	say SUCCESS "$VERSION is free on crates.io for both crates"
}

only_release_files_changed() {
	local line path allowed file
	while IFS= read -r line; do
		[[ -n "$line" ]] || continue
		path="${line:3}"
		allowed=0
		while IFS= read -r file; do
			[[ "$path" == "$file" ]] && allowed=1
		done < <(tracked_release_files)
		[[ $allowed -eq 1 ]] || return 1
	done < <(git status --porcelain)
	return 0
}

write_attribution() {
	run "cargo about generate" cargo about generate about.hbs -o "$ATTRIBUTION"
	[[ -s "$ATTRIBUTION" ]] || die "$ATTRIBUTION came out empty"
}

tracked_release_files() {
	local file
	for file in "$ROOT_MANIFEST" "$CLI_MANIFEST" "$LOCK_FILE" "$CHANGELOG" "$MCPB_MANIFEST" "$REGISTRY_MANIFEST"; do
		[[ -f "$file" ]] && printf '%s\n' "$file"
	done
	return 0
}

snapshot_tree() {
	local file
	while IFS= read -r file; do
		mkdir -p -- "$WORK_DIR/backup/$(dirname -- "$file")"
		cp -p -- "$file" "$WORK_DIR/backup/$file"
	done < <(tracked_release_files)
	MUTATED=1
}

restore_tree() {
	local file
	while IFS= read -r file; do
		if [[ -f "$WORK_DIR/backup/$file" ]]; then
			cp -p -- "$WORK_DIR/backup/$file" "$file"
		fi
	done < <(tracked_release_files)
	MUTATED=0
}

replace_atomically() {
	local target="$1" source="$2" dir temp
	dir="$(dirname -- "$target")"
	temp="$(mktemp "$dir/.replace.XXXXXX")" || die "could not create a temp file for $target"
	cat -- "$source" >"$temp"
	mv -f -- "$temp" "$target"
}

rewrite_file() {
	local target="$1" program="$2"
	shift 2
	local temp="$WORK_DIR/rewrite.tmp" code=0
	awk "$@" -f "$program" "$target" >"$temp" || code=$?
	if [[ $code -ne 0 ]]; then
		rm -f -- "$temp"
		return "$code"
	fi
	if cmp -s -- "$temp" "$target"; then
		rm -f -- "$temp"
		return 0
	fi
	replace_atomically "$target" "$temp"
	rm -f -- "$temp"
}

bump_workspace_version() {
	local workspace_program="$WORK_DIR/bump-workspace.awk"
	cat >"$workspace_program" <<-'AWK'
		/^\[/ { section = $0 }
		{
			if (!done && section == "[workspace.package]" && $0 ~ /^version[[:space:]]*=/) {
				sub(/"[^"]*"/, "\"" new "\"")
				done = 1
			}
			print
		}
		END { if (!done) exit 3 }
	AWK
	rewrite_file "$ROOT_MANIFEST" "$workspace_program" -v new="$VERSION" ||
		die "no version line under [workspace.package] in $ROOT_MANIFEST"
	say SUCCESS "$ROOT_MANIFEST [workspace.package] version is $VERSION"

	local dependency_program="$WORK_DIR/bump-dependency.awk"
	cat >"$dependency_program" <<-'AWK'
		{
			if (!done && $0 ~ /^ownpg-core[[:space:]]*=/) {
				if (sub(/version[[:space:]]*=[[:space:]]*"[^"]*"/, "version = \"" new "\"")) done = 1
			}
			print
		}
		END { if (!done) exit 3 }
	AWK
	rewrite_file "$CLI_MANIFEST" "$dependency_program" -v new="$VERSION" ||
		die "no ownpg-core dependency version in $CLI_MANIFEST"
	say SUCCESS "$CLI_MANIFEST ownpg-core dependency is $VERSION"

	bump_json_manifest "$MCPB_MANIFEST" '.version'
	bump_json_manifest "$REGISTRY_MANIFEST" '.version' '.packages[] | select(.registryType == "cargo") | .version'
}

bump_json_manifest() {
	local file="$1" staged="$WORK_DIR/json-bump.tmp"
	shift
	if json_versions_match "$file" "$VERSION" "$@"; then
		say INFO "$file already carries $VERSION, left as it is"
		return 0
	fi
	set_json_versions "$file" "$VERSION" "$staged" "$@" ||
		die "could not change only the version fields of $file to $VERSION; set them by hand"
	replace_atomically "$file" "$staged"
	rm -f -- "$staged"
	say SUCCESS "$file version is $VERSION"
}

move_changelog_section() {
	local today program="$WORK_DIR/changelog.awk"
	if [[ ! -f "$CHANGELOG" ]]; then
		say WARNING "$CHANGELOG does not exist; the release notes come from the commit subjects since the previous tag"
		return 0
	fi
	if grep -qF "## [$VERSION]" "$CHANGELOG"; then
		say INFO "$CHANGELOG already has a $VERSION section"
		add_changelog_link
		advance_unreleased_link
		return 0
	fi
	today="$(date +%F)"
	cat >"$program" <<-'AWK'
		!done && /^## \[Unreleased\]/ {
			print "## [Unreleased]"
			print ""
			print "## [" new "] - " today
			done = 1
			next
		}
		{ print }
		END { if (!done) exit 3 }
	AWK
	if rewrite_file "$CHANGELOG" "$program" -v new="$VERSION" -v today="$today"; then
		say SUCCESS "$CHANGELOG now records $VERSION on $today"
	else
		say WARNING "$CHANGELOG has no [Unreleased] section; write the $VERSION entry by hand"
		return 0
	fi
	add_changelog_link
	advance_unreleased_link
}

advance_unreleased_link() {
	local program="$WORK_DIR/changelog-unreleased.awk"
	cat >"$program" <<-'AWK'
		/^\[Unreleased\]: / { sub(/\/compare\/v[0-9]+\.[0-9]+\.[0-9]+\.\.\.HEAD$/, "/compare/v" new "...HEAD") }
		{ print }
	AWK
	rewrite_file "$CHANGELOG" "$program" -v new="$VERSION" ||
		die "could not point the [Unreleased] link of $CHANGELOG at v$VERSION"
}

add_changelog_link() {
	local link="[$VERSION]: $RELEASE_URL_BASE/v$VERSION" program="$WORK_DIR/changelog-link.awk"
	if grep -qF "[$VERSION]: " "$CHANGELOG"; then
		say INFO "$CHANGELOG already carries a link reference for $VERSION"
		return 0
	fi
	cat >"$program" <<-'AWK'
		{ print }
		!done && /^\[Unreleased\]: / { print link; done = 1 }
		END { if (!done) print link }
	AWK
	rewrite_file "$CHANGELOG" "$program" -v link="$link" ||
		die "could not add the link reference for $VERSION to $CHANGELOG"
	say SUCCESS "$CHANGELOG links $VERSION to $RELEASE_URL_BASE/v$VERSION"
}

tag_flag() {
	if [[ -n "$(git config --get user.signingkey || true)" ]]; then
		printf -- '-s'
	else
		printf -- '-a'
	fi
}

confirm() {
	local want="$1"
	if [[ $ASSUME_YES -eq 1 ]]; then
		say WARNING "--yes was given, so the confirmation is skipped"
		return 0
	fi
	[[ -t 0 ]] || die "no terminal to confirm on; pass --yes for an unattended run"
	local typed=""
	printf 'type %s to continue, anything else stops here: ' "$want"
	IFS= read -r typed || true
	[[ "$typed" == "$want" ]] || die "that did not match $want; nothing was published"
}

publish_crate() {
	local name="$1" log="$WORK_DIR/publish-$1.log" code=0
	STEP="publish $name"
	say INFO "publishing $name $VERSION"
	cargo publish -p "$name" --locked --allow-dirty --color=never >"$log" 2>&1 || code=$?
	if [[ $code -ne 0 ]]; then
		if grep -qiE 'already exists|already uploaded|already been uploaded' "$log"; then
			say WARNING "$name $VERSION is already on crates.io, continuing"
			return 0
		fi
		if published_already "$name"; then
			IRREVERSIBLE=1
			say WARNING "$name $VERSION did reach crates.io despite the failure"
		elif [[ $UPLOADED -eq 0 ]]; then
			IRREVERSIBLE=0
			say INFO "$name $VERSION did not reach crates.io; nothing was published"
		fi
		say INFO "last 40 lines of output:"
		tail -n 40 "$log" >&2
		die "publishing $name failed (exit $code)"
	fi
	UPLOADED=1
	say SUCCESS "published $name $VERSION"
}

dist_targets() {
	local plan="$WORK_DIR/dist-plan.json" log="$WORK_DIR/dist-plan.log" code=0
	dist plan --tag="v$VERSION" --output-format=json >"$plan" 2>"$log" || code=$?
	if [[ $code -ne 0 ]]; then
		say INFO "last 40 lines of output:"
		tail -n 40 "$log" >&2
		die "dist plan failed (exit $code) for v$VERSION"
	fi
	jq -er '.ci.github.artifacts_matrix.include[].targets[]' "$plan" ||
		die "dist plan produced JSON without .ci.github.artifacts_matrix.include[].targets[]; the dist output schema may have changed"
}

c_flags_for() {
	case "$1" in
	aarch64-pc-windows-msvc) printf -- '-Wno-error=incompatible-pointer-types -D_mm_pause=__builtin_arm_yield -U__ARM_NEON' ;;
	*-pc-windows-msvc) printf -- '-Wno-error=incompatible-pointer-types' ;;
	*) printf '' ;;
	esac
}

xwin_compiler_for() {
	case "$1" in
	aarch64-pc-windows-msvc) printf 'clang' ;;
	*) printf 'clang-cl' ;;
	esac
}

skip_reason() {
	if grep -qF 'tools are required to run this task, but are missing' "$1"; then
		printf 'missing cross-compile tool'
	elif grep -qE 'linker .* not found|error: linking with .* failed|error: toolchain .* does not support|no such file or directory.*(gcc|cc1|link\.exe)' "$1"; then
		printf 'missing linker'
	else
		return 1
	fi
}

build_dist_artifacts() {
	export XWIN_INCLUDE_DEBUG_SYMBOLS=true
	local repo_root
	repo_root="$(pwd)"
	export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$repo_root=/ownpg --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo"
	SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)"
	export SOURCE_DATE_EPOCH
	rm -rf -- target/distrib
	local target log manifest reason
	local -a targets=() built=() skipped=()
	while IFS= read -r target; do
		[[ -n "$target" ]] && targets+=("$target")
	done < <(dist_targets)
	[[ ${#targets[@]} -gt 0 ]] || die "dist plan returned no targets to build"
	say INFO "building local artifacts for: ${targets[*]}"
	for target in "${targets[@]}"; do
		log="$WORK_DIR/dist-build-$target.log"
		manifest="$WORK_DIR/dist-build-$target.json"
		if CFLAGS="$(c_flags_for "$target")" XWIN_CROSS_COMPILER="$(xwin_compiler_for "$target")" dist build --tag="v$VERSION" --artifacts=local --target="$target" --no-local-paths \
			--output-format=json >"$manifest" 2>"$log"; then
			mkdir -p -- target/distrib
			cp -- "$manifest" "target/distrib/$target-dist-manifest.json"
			built+=("$target")
			say SUCCESS "built $target"
		elif reason="$(skip_reason "$log")"; then
			skipped+=("$target")
			say WARNING "$target skipped ($reason); last 15 lines:"
			tail -n 15 "$log" | sed 's/^/    /'
		else
			say INFO "last 40 lines of output:"
			tail -n 40 "$log" >&2
			die "$target failed to build and it does not look like a missing cross toolchain; this may be a real defect, not something safe to skip"
		fi
	done
	[[ ${#built[@]} -gt 0 ]] || die "no target built; nothing to release"
	if [[ ${#skipped[@]} -gt 0 ]]; then
		say WARNING "skipped target(s): ${skipped[*]}"
		say WARNING "the shell and PowerShell installers still offer every configured target, so"
		say WARNING "a user on a skipped target gets a failed download, not a clear message"
		[[ "${OWNPG_ALLOW_PARTIAL:-0}" == "1" ]] ||
			die "built ${#built[@]} of ${#targets[@]} targets; install the missing cross toolchains, or set OWNPG_ALLOW_PARTIAL=1 to publish this partial set on purpose (crates.io and the tag are already live either way)"
		if [[ $ASSUME_YES -ne 1 ]]; then
			[[ -t 0 ]] || die "no terminal to confirm a partial target set on; pass --yes for an unattended run"
			local typed=""
			printf 'built %s of %s targets (%s); type %s to publish this partial set, anything else stops here: ' \
				"${#built[@]}" "${#targets[@]}" "${built[*]}" "$VERSION"
			IFS= read -r typed || true
			[[ "$typed" == "$VERSION" ]] ||
				die "that did not match $VERSION; crates.io and the tag are already live, finish the binaries by hand when ready"
		fi
	fi
	smoke_test_host_archive "${built[@]}"
	sign_macos_binaries "${built[@]}"
	sign_windows_binaries "${built[@]}"
	build_mcpb_bundles "${built[@]}"
	run "dist build --artifacts=global" dist build --tag="v$VERSION" --artifacts=global --no-local-paths
	cp -- "$ATTRIBUTION" target/distrib/ || die "could not place $ATTRIBUTION next to the archives"
}

host_triple() {
	rustc -vV | awk '/^host:/ { print $2 }'
}

smoke_test_host_archive() {
	local host target archive stage printed
	host="$(host_triple)"
	for target in "$@"; do
		[[ "$target" == "$host" ]] || continue
		archive="target/distrib/$BIN_CRATE-$target.tar.gz"
		[[ -f "$archive" ]] || die "$archive is missing; dist did not produce the archive for $host"
		stage="$WORK_DIR/smoke-$target"
		rm -rf -- "$stage"
		mkdir -p -- "$stage"
		tar -xzf "$archive" -C "$stage" || die "could not unpack $archive for the smoke test"
		printed="$("$stage/$BIN_CRATE-$target/$BIN_CRATE" --version 2>&1 || true)"
		[[ "$printed" == "$BIN_CRATE $VERSION ("* ]] ||
			die "the shipped $target binary prints '$printed', expected '$BIN_CRATE $VERSION (commit ..., built ...)'"
		[[ "$printed" != *"commit unknown"* && "$printed" != *"built unknown"* ]] ||
			die "the shipped $target binary carries an unknown build stamp: $printed"
		say SUCCESS "the shipped $target archive runs and reports $VERSION"
		return 0
	done
	say WARNING "no archive was built for this host ($host); the shipped binary was not run here"
}

sign_macos_binaries() {
	local identity="${OWNPG_CODESIGN_IDENTITY:-}" profile="${OWNPG_NOTARY_PROFILE:-}"
	if [[ -z "$identity" || -z "$profile" ]]; then
		say WARNING "OWNPG_CODESIGN_IDENTITY or OWNPG_NOTARY_PROFILE is unset; the macOS binaries ship unsigned and Gatekeeper will warn on first run"
		return 0
	fi
	[[ "$(uname -s)" == "Darwin" ]] || die "macOS signing needs codesign and notarytool, which only exist on macOS"
	require_tools codesign xcrun ditto
	local target archive stage zip
	for target in "$@"; do
		[[ "$target" == *-apple-darwin ]] || continue
		archive="target/distrib/$BIN_CRATE-$target.tar.gz"
		[[ -f "$archive" ]] || die "$archive is missing; dist did not produce the macOS archive for $target"
		stage="$WORK_DIR/sign-$target"
		rm -rf -- "$stage"
		mkdir -p -- "$stage"
		tar -xzf "$archive" -C "$stage" || die "could not unpack $archive"
		run "codesign ($target)" codesign --sign "$identity" --timestamp --options=runtime --force "$stage/$BIN_CRATE-$target/$BIN_CRATE"
		run "codesign --verify ($target)" codesign --verify --strict --verbose=2 "$stage/$BIN_CRATE-$target/$BIN_CRATE"
		zip="$WORK_DIR/$BIN_CRATE-$target.zip"
		rm -f -- "$zip"
		run "ditto ($target)" ditto -c -k --keepParent "$stage/$BIN_CRATE-$target/$BIN_CRATE" "$zip"
		run "notarytool submit ($target)" xcrun notarytool submit "$zip" --keychain-profile "$profile" --wait
		tar -czf "$archive" -C "$stage" "$BIN_CRATE-$target" || die "could not repack $archive after signing"
		say SUCCESS "signed and notarized $target"
	done
}

sign_windows_binaries() {
	local cert="${OWNPG_WINDOWS_SIGN_CERT:-}" pass="${OWNPG_WINDOWS_SIGN_PASS:-}"
	if [[ -z "$cert" || -z "$pass" ]]; then
		say WARNING "OWNPG_WINDOWS_SIGN_CERT or OWNPG_WINDOWS_SIGN_PASS is unset; the Windows binaries ship unsigned and SmartScreen will warn on first run"
		return 0
	fi
	[[ -f "$cert" ]] || die "OWNPG_WINDOWS_SIGN_CERT points at $cert, which does not exist"
	require_tools osslsigncode
	local pass_file="$WORK_DIR/windows-sign.pass"
	(umask 077 && printf '%s' "$pass" >"$pass_file") || die "could not stage the signing password"
	unset OWNPG_WINDOWS_SIGN_PASS pass
	local target archive stage exe
	for target in "$@"; do
		[[ "$target" == *-pc-windows-msvc ]] || continue
		archive="target/distrib/$BIN_CRATE-$target.zip"
		[[ -f "$archive" ]] || die "$archive is missing; dist did not produce the Windows archive for $target"
		stage="$WORK_DIR/sign-$target"
		rm -rf -- "$stage"
		mkdir -p -- "$stage"
		unzip -q "$archive" -d "$stage" || die "could not unpack $archive"
		exe="$(find "$stage" -type f -name "$BIN_CRATE.exe" | head -n 1)"
		[[ -n "$exe" ]] || die "$archive holds no $BIN_CRATE.exe"
		run "osslsigncode ($target)" osslsigncode sign -pkcs12 "$cert" -readpass "$pass_file" -n "OwnPG" -i "$RELEASE_URL_BASE" -t http://timestamp.digicert.com -in "$exe" -out "$exe.signed"
		mv -f -- "$exe.signed" "$exe" || die "could not replace $exe with the signed binary"
		run "osslsigncode verify ($target)" osslsigncode verify -in "$exe"
		rm -f -- "$archive"
		(cd "$stage" && zip -q -r "$REPO/$archive" .) || die "could not repack $archive after signing"
		say SUCCESS "signed $target"
	done
	rm -f -- "$pass_file"
}

publish_homebrew_formula() {
	local tap="${OWNPG_HOMEBREW_TAP:-devops-infinity/homebrew-tap}" formula="target/distrib/$BIN_CRATE.rb" clone
	if [[ "${OWNPG_SKIP_TAP:-0}" == "1" ]]; then
		say INFO "OWNPG_SKIP_TAP=1; the formula stays at $formula"
		return 0
	fi
	[[ -f "$formula" ]] || die "$formula is missing; dist did not write the Homebrew formula"
	clone="$WORK_DIR/homebrew-tap"
	rm -rf -- "$clone"
	run "gh repo clone $tap" gh repo clone "$tap" "$clone" -- --quiet --depth 1
	mkdir -p -- "$clone/Formula"
	cp -- "$formula" "$clone/Formula/$BIN_CRATE.rb" || die "could not copy the formula into the tap"
	if git -C "$clone" diff --quiet -- "Formula/$BIN_CRATE.rb" && [[ -z "$(git -C "$clone" status --porcelain -- "Formula/$BIN_CRATE.rb")" ]]; then
		say INFO "the tap already carries this formula"
		return 0
	fi
	git -C "$clone" add -- "Formula/$BIN_CRATE.rb" || die "could not stage the formula"
	git -C "$clone" -c user.name="$(git config user.name)" -c user.email="$(git config user.email)" commit --quiet -m "$BIN_CRATE $VERSION" || die "could not commit the formula"
	run "git push ($tap)" git -C "$clone" push --quiet origin HEAD
	say SUCCESS "formula pushed to $tap"
}

publish_npm_package() {
	local package="target/distrib/$BIN_CRATE-npm-package.tar.gz"
	if [[ "${OWNPG_SKIP_NPM:-0}" == "1" ]]; then
		say INFO "OWNPG_SKIP_NPM=1; the npm package stays at $package"
		return 0
	fi
	[[ -f "$package" ]] || die "$package is missing; dist did not write the npm package"
	local package_name log
	package_name="$(tar -xOzf "$package" package/package.json | jq -r '.name')" ||
		die "could not read the package name from $package"
	if [[ "$(npm view "$package_name@$VERSION" version --color=false 2>/dev/null || true)" == "$VERSION" ]]; then
		say INFO "$package_name $VERSION is already on npm"
		return 0
	fi
	STEP="npm publish"
	log="$(log_path_for "npm publish")"
	if ! npm publish "$package" --access public >"$log" 2>&1; then
		grep -q 'EOTP' "$log" &&
			die "npm wants a one-time password for $package_name; ~/.npmrc needs a publish token that bypasses 2FA, or finish by hand with: npm publish $package --access public"
		say INFO "last 40 lines of output:"
		tail -n 40 "$log" >&2
		die "npm publish failed for $package_name $VERSION"
	fi
	say SUCCESS "npm package published"
}

mcpb_platform() {
	case "$1" in
	aarch64-apple-darwin) printf 'darwin arm64\n' ;;
	x86_64-apple-darwin) printf 'darwin x64\n' ;;
	x86_64-unknown-linux-gnu) printf 'linux x64\n' ;;
	aarch64-unknown-linux-gnu) printf 'linux arm64\n' ;;
	x86_64-pc-windows-msvc) printf 'win32 x64\n' ;;
	aarch64-pc-windows-msvc) printf 'win32 arm64\n' ;;
	*) return 1 ;;
	esac
}

build_mcpb_bundles() {
	local target archive platform arch stage bundle binary
	local -a bundles=()
	for target in "$@"; do
		if ! read -r platform arch < <(mcpb_platform "$target" || true); then
			say INFO "$target has no MCPB platform; skipping the bundle"
			continue
		fi
		stage="$WORK_DIR/mcpb-$target"
		rm -rf -- "$stage"
		mkdir -p -- "$stage/bin"
		case "$platform" in
		win32)
			archive="target/distrib/$BIN_CRATE-$target.zip"
			binary="$BIN_CRATE.exe"
			[[ -f "$archive" ]] || die "$archive is missing; dist did not produce the Windows archive for $target"
			unzip -q -o -j "$archive" "$binary" -d "$stage/bin" || die "could not extract $binary from $archive"
			;;
		*)
			archive="target/distrib/$BIN_CRATE-$target.tar.gz"
			binary="$BIN_CRATE"
			[[ -f "$archive" ]] || die "$archive is missing; dist did not produce the archive for $target"
			tar -xzf "$archive" -C "$stage/bin" --strip-components=1 "$BIN_CRATE-$target/$binary" || die "could not extract $binary from $archive"
			chmod 0755 "$stage/bin/$binary"
			;;
		esac
		jq --arg version "$VERSION" --arg platform "$platform" --arg entry "bin/$binary" \
			'.version = $version | .compatibility.platforms = [$platform] | .server.entry_point = $entry | .server.mcp_config.command = ("${__dirname}/" + $entry)' \
			mcpb/manifest.json >"$stage/manifest.json" || die "could not write the manifest for $target"
		run "mcpb validate ($target)" mcpb validate "$stage/manifest.json"
		bundle="target/distrib/$BIN_CRATE-$VERSION-$platform-$arch.mcpb"
		run "mcpb pack ($target)" mcpb pack "$stage" "$bundle"
		[[ -f "$bundle" ]] || die "mcpb did not write $bundle"
		bundles+=("$bundle")
	done
	[[ ${#bundles[@]} -gt 0 ]] || die "no MCPB bundle was built"
	say SUCCESS "built ${#bundles[@]} MCPB bundle(s)"
}

write_registry_manifest() {
	local bundle name sha packages
	packages="$(jq -c --arg version "$VERSION" '.packages | map(select(.registryType == "cargo") | .version = $version)' "$REGISTRY_MANIFEST")"
	shopt -s nullglob
	for bundle in target/distrib/*.mcpb; do
		name="$(basename "$bundle")"
		sha="$(shasum -a 256 "$bundle" | awk '{print $1}')"
		packages="$(jq -c --arg version "$VERSION" --arg url "https://github.com/$PUBLIC_RELEASE_REPO/releases/download/v$VERSION/$name" --arg sha "$sha" \
			'. + [{registryType: "mcpb", identifier: $url, version: $version, fileSha256: $sha, transport: {type: "stdio"}}]' <<<"$packages")"
	done
	shopt -u nullglob
	jq --arg version "$VERSION" --argjson packages "$packages" '.version = $version | .packages = $packages' "$REGISTRY_MANIFEST" >"target/distrib/$REGISTRY_MANIFEST" || die "could not write the filled $REGISTRY_MANIFEST"
	say SUCCESS "target/distrib/$REGISTRY_MANIFEST carries $VERSION and $(jq '.packages | length' "target/distrib/$REGISTRY_MANIFEST") package entries"
}

sign_checksums() {
	local sums="target/distrib/sha256.sum"
	[[ -f "$sums" ]] || die "$sums is missing; the global build did not produce a checksum file"
	STEP="minisign -Sm $sums"
	if [[ -t 0 ]]; then
		minisign -Sm "$sums" -s "$MINISIGN_KEY" -t "OwnPG $VERSION" </dev/tty || die "minisign failed"
	else
		minisign -Sm "$sums" -s "$MINISIGN_KEY" -t "OwnPG $VERSION" || die "minisign failed; it needs a terminal to ask for the key password"
	fi
	[[ -f "$sums.minisig" ]] || die "minisign did not write $sums.minisig"
	say SUCCESS "signed $sums"
	if [[ -f "$MINISIGN_PUB" ]]; then
		cp -- "$MINISIGN_PUB" target/distrib/minisign.pub || die "could not place the minisign public key next to the checksums"
		say SUCCESS "minisign.pub ships next to sha256.sum.minisig"
	else
		say WARNING "$MINISIGN_PUB is missing; users get a signature they cannot check until the public key is committed at the repository root"
	fi
}

release_notes_from_git() {
	local previous
	previous="$(git describe --tags --abbrev=0 --match 'v[0-9]*' HEAD^ 2>/dev/null || true)"
	if [[ -n "$previous" ]]; then
		git log --no-merges --format='- %s' "$previous..HEAD"
	else
		git log --no-merges --format='- %s'
	fi
}

changelog_section() {
	awk -v want="$VERSION" '
		BEGIN { gsub(/[.]/, "\\.", want) }
		/^## \[/ {
			if (printing) exit
			printing = ($0 ~ ("\\[" want "\\]"))
			next
		}
		printing && /^\[[^]]*\]:/ { exit }
		printing { print }
	' "$CHANGELOG"
}

changelog_section_problem() {
	local heading_count
	heading_count="$(grep -cF "## [$VERSION]" "$CHANGELOG" || true)"
	if [[ "$heading_count" -ne 1 ]]; then
		printf '%s headings for [%s], expected exactly one' "$heading_count" "$VERSION"
		return 0
	fi
	if ! changelog_section | grep -q '[^[:space:]]'; then
		printf 'the [%s] section is empty' "$VERSION"
		return 0
	fi
	return 1
}

publish_github_release() {
	local target_repo="$1"
	local -a repo_flag=()
	[[ -z "$target_repo" ]] || repo_flag=(--repo "$target_repo")
	local where="${target_repo:-this repository}"

	local -a assets=() entries=()
	local entry notes="$WORK_DIR/release-notes.md"
	shopt -s nullglob
	entries=(target/distrib/*)
	shopt -u nullglob
	for entry in "${entries[@]}"; do
		[[ -f "$entry" && "$entry" != *-dist-manifest.json ]] && assets+=("$entry")
	done
	[[ ${#assets[@]} -gt 0 ]] || die "target/distrib has no files; nothing to upload"

	local problem
	if [[ -f "$CHANGELOG" ]]; then
		if problem="$(changelog_section_problem)"; then
			die "$CHANGELOG: $problem; fix it by hand before the release notes can be trusted"
		fi
		changelog_section >"$notes"
	else
		release_notes_from_git >"$notes"
		grep -q '[^[:space:]]' "$notes" || die "no commits since the previous tag; nothing to write into the release notes"
	fi

	local view="$WORK_DIR/gh-view-${target_repo//\//-}.txt" view_err="$WORK_DIR/gh-view-${target_repo//\//-}.err"
	if gh release view "v$VERSION" "${repo_flag[@]}" --json assets --jq '.assets[].name' >"$view" 2>"$view_err"; then
		local name
		local -a have_names=() want_names=()
		while IFS= read -r name; do
			[[ -n "$name" ]] && have_names+=("$name")
		done < <(sort "$view")
		while IFS= read -r name; do
			[[ -n "$name" ]] && want_names+=("$name")
		done < <(printf '%s\n' "${assets[@]##*/}" | sort)
		if [[ "${have_names[*]}" == "${want_names[*]}" ]]; then
			say WARNING "GitHub release v$VERSION on $where already carries every asset just built, leaving it alone"
			return 0
		fi
		say WARNING "GitHub release v$VERSION on $where exists but its assets do not match what was just built"
		say INFO "  on the release: ${have_names[*]:-none}"
		say INFO "  built here:     ${want_names[*]}"
		if [[ $RESUME_REQUESTED -eq 1 ]]; then
			run "gh release upload v$VERSION on $where" gh release upload "v$VERSION" "${assets[@]}" "${repo_flag[@]}" --clobber
			say SUCCESS "GitHub release v$VERSION on $where now carries the full set built here, checksums and signature included"
			return 0
		fi
		die "refusing to leave a mismatched release in place on $where; settle it with 'gh release upload v$VERSION \$(find target/distrib -maxdepth 1 -type f) ${repo_flag[*]} --clobber' or delete v$VERSION there and rerun"
	elif ! grep -qi 'release not found\|not found' "$view_err"; then
		say INFO "last 40 lines of output:"
		tail -n 40 "$view_err" >&2
		die "could not ask $where whether v$VERSION already exists; not creating a release blind"
	fi

	local label="gh release create v$VERSION on $where"
	local log
	log="$(log_path_for "$label")"
	run "$label" gh release create "v$VERSION" "${assets[@]}" "${repo_flag[@]}" \
		--title "OwnPG $VERSION" --notes-file "$notes"
	local release_url
	release_url="$(grep -oE 'https://github\.com/[^[:space:]]+/releases/tag/[^[:space:]]+' "$log" | tail -n 1 || true)"
	[[ -z "$release_url" ]] || say SUCCESS "release page: $release_url"
}

wait_for_crate() {
	local name="$1" tries=0 started=$SECONDS
	say INFO "waiting for $name $VERSION to show up on crates.io"
	while [[ $tries -lt $PROPAGATE_TRIES ]]; do
		fetch_crate "$name"
		if [[ "$CRATE_HTTP_STATUS" == "200" ]] && crate_has_version "$VERSION"; then
			say SUCCESS "$name $VERSION is live"
			return 0
		fi
		tries=$((tries + 1))
		sleep "$((PROPAGATE_WAIT + RANDOM % PROPAGATE_JITTER))"
	done
	die "$name $VERSION did not appear after $((SECONDS - started))s; publish $BIN_CRATE by hand once it does"
}

check_semver() {
	fetch_crate "$LIB_CRATE"
	if [[ "$CRATE_HTTP_STATUS" != "200" ]]; then
		say INFO "cargo semver-checks skipped: $LIB_CRATE is not on crates.io yet"
		return 0
	fi
	run "cargo semver-checks" cargo semver-checks check-release -p "$LIB_CRATE" --color never
}

yank_crates() {
	local action_word="$1"
	local action="yank"
	local -a order=("$BIN_CRATE" "$LIB_CRATE")
	if [[ "$action_word" == "unyank" ]]; then
		action="unyank"
		order=("$LIB_CRATE" "$BIN_CRATE")
	fi

	require_tools cargo
	require_credentials

	say INFO "$action $VERSION from ${order[0]} then ${order[1]}"
	if [[ "$action" == "yank" ]]; then
		say WARNING "a yank does not delete anything"
		say WARNING "it stops new resolution; a lockfile that already names $VERSION still fetches it"
		say WARNING "the source stays downloadable from crates.io"
	fi
	confirm "$VERSION"

	local name code
	local -a command
	for name in "${order[@]}"; do
		code=0
		command=(cargo yank "$name" --version "$VERSION")
		if [[ "$action_word" == "unyank" ]]; then
			command+=(--undo)
		fi
		"${command[@]}" >"$WORK_DIR/$action-$name.log" 2>&1 || code=$?
		if [[ $code -ne 0 ]]; then
			say INFO "last 40 lines of output:"
			tail -n 40 "$WORK_DIR/$action-$name.log" >&2
			die "$action of $name $VERSION failed (exit $code)"
		fi
		say SUCCESS "$action $name $VERSION"
	done
}

prepare_release() {
	require_tools git cargo jq awk cmp
	[[ -z "$(git status --porcelain)" ]] || die "the working tree is not clean; commit or stash first"
	CURRENT_VERSION="$(read_current_version)"
	valid_version "$CURRENT_VERSION" || die "the version in $ROOT_MANIFEST is not a semver triple: $CURRENT_VERSION"
	if [[ -n "$BUMP" ]]; then
		VERSION="$(next_version "$CURRENT_VERSION" "$BUMP")" || die "unknown bump: $BUMP"
	fi
	version_greater "$VERSION" "$CURRENT_VERSION" || die "$VERSION is not greater than the current $CURRENT_VERSION"
	step "version bump"
	bump_workspace_version
	run "cargo check after the bump" cargo check --workspace --all-targets --all-features --color=never
	say SUCCESS "$LOCK_FILE records $VERSION"
	step "changelog"
	move_changelog_section
	say SUCCESS "the $CURRENT_VERSION -> $VERSION bump is in the working tree; commit it on a branch, merge it through a pull request, then run tools/release.sh --version $VERSION from $BRANCH"
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--version)
		[[ $# -ge 2 ]] || die "--version needs X.Y.Z"
		VERSION="$2"
		shift 2
		;;
	--major | --minor | --patch)
		BUMP="${1#--}"
		shift
		;;
	--dry-run)
		MODE="dry-run"
		shift
		;;
	--prepare)
		MODE="prepare"
		shift
		;;
	--resume)
		RESUME_REQUESTED=1
		shift
		;;
	--yes)
		ASSUME_YES=1
		shift
		;;
	--yank)
		[[ $# -ge 2 ]] || die "--yank needs X.Y.Z"
		MODE="yank"
		VERSION="$2"
		shift 2
		;;
	--unyank)
		[[ $# -ge 2 ]] || die "--unyank needs X.Y.Z"
		MODE="unyank"
		VERSION="$2"
		shift 2
		;;
	--branch)
		[[ $# -ge 2 ]] || die "--branch needs a branch name"
		BRANCH="$2"
		shift 2
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		usage
		die "unknown option: $1"
		;;
	esac
done

cd "$REPO"

if [[ -n "$BUMP" && "$MODE" != "release" && "$MODE" != "dry-run" && "$MODE" != "prepare" ]]; then
	die "--$BUMP names a new version, so it cannot be combined with --$MODE"
fi
if [[ -n "$BUMP" && -n "$VERSION" ]]; then
	die "give either --$BUMP or --version, not both"
fi
if [[ -z "$BUMP" && -z "$VERSION" ]]; then
	usage
	die "say which release: --major, --minor, --patch, or --version X.Y.Z"
fi
if [[ -n "$VERSION" ]]; then
	valid_version "$VERSION" || die "not a semver triple: $VERSION"
fi
if [[ $RESUME_REQUESTED -eq 1 ]]; then
	[[ "$MODE" == "release" ]] || die "--resume finishes a real release, so it cannot be combined with --$MODE"
	[[ -n "$VERSION" ]] || die "--resume needs the version being finished: --resume --version X.Y.Z"
fi

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ownpg-release.XXXXXX")"

say INFO "repository $REPO"

if [[ "$MODE" == "prepare" ]]; then
	prepare_release
	exit 0
fi

RELEASE_LOCK="$REPO/.git/release.lock"
acquire_pid_lock "$RELEASE_LOCK" ||
	die "another release.sh is already running (lock held: $RELEASE_LOCK, process $(cat -- "$RELEASE_LOCK/pid" 2>/dev/null || printf 'unknown')); remove it by hand only if you are sure none is running"
LOCK_DIR="$RELEASE_LOCK"

case "$MODE" in
yank)
	yank_crates yank
	exit 0
	;;
unyank)
	yank_crates unyank
	exit 0
	;;
*) ;;
esac

step "pre-flight"
require_tools git cargo curl jq awk shasum unzip tar cargo-nextest cargo-audit cargo-deny cargo-machete cargo-about cargo-auditable cargo-cyclonedx cargo-semver-checks dist gh minisign mcpb
require_tool_version jq "$(jq --version)" "$JQ_FLOOR" "install it with: brew install jq"
require_tool_version curl "$(curl --version | awk 'NR == 1 { print $2 }')" "$CURL_FLOOR" \
	"install it with: brew install curl, then put \$(brew --prefix curl)/bin ahead of /usr/bin on PATH"
if [[ -z "$(git status --porcelain)" ]]; then
	say SUCCESS "working tree is clean"
elif [[ $RESUME_REQUESTED -eq 1 ]] && only_release_files_changed; then
	say WARNING "the working tree carries the uncommitted version bump from the stopped run"
else
	die "the working tree is not clean; commit or stash first"
fi
CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$CURRENT_BRANCH" == "$BRANCH" ]] || die "on branch $CURRENT_BRANCH; a release is cut from $BRANCH"
say SUCCESS "on $BRANCH"
git fetch --quiet "$REMOTE" "$BRANCH" || die "could not fetch $REMOTE/$BRANCH"
BEHIND="$(git rev-list --count "HEAD..$REMOTE/$BRANCH")"
[[ "$BEHIND" -eq 0 ]] || die "$BRANCH is $BEHIND commit(s) behind $REMOTE/$BRANCH; pull first"
say SUCCESS "$BRANCH is level with $REMOTE/$BRANCH"
require_credentials
require_gh_auth
require_npm_auth
require_minisign_key
check_dist_version
require_llvm_tools
CURRENT_VERSION="$(read_current_version)"
[[ -n "$CURRENT_VERSION" ]] || die "could not read the version from $ROOT_MANIFEST"
valid_version "$CURRENT_VERSION" || die "the version in $ROOT_MANIFEST is not a semver triple: $CURRENT_VERSION"
if [[ -n "$BUMP" ]]; then
	VERSION="$(next_version "$CURRENT_VERSION" "$BUMP")" || die "unknown bump: $BUMP"
	say INFO "$BUMP release: $CURRENT_VERSION -> $VERSION"
fi
check_registry_state
if [[ "$VERSION" == "$CURRENT_VERSION" ]]; then
	say INFO "the manifests already carry $VERSION, so the bump step changes nothing"
else
	version_greater "$VERSION" "$CURRENT_VERSION" || die "$VERSION is not greater than the current $CURRENT_VERSION"
	say SUCCESS "$CURRENT_VERSION -> $VERSION"
fi
if [[ "$MODE" == "release" ]] && branch_is_protected "$BRANCH"; then
	require_prepared_release
fi

step "verification gate"
run "tools/verify.sh" "$REPO/tools/verify.sh"
check_semver
write_attribution

step "packaging proof"
if [[ $LIB_PUBLISHED -eq 1 ]]; then
	say INFO "$LIB_CRATE $VERSION is already on crates.io, so its packaging proof is skipped"
else
	run "cargo publish --dry-run -p $LIB_CRATE" cargo publish --dry-run -p "$LIB_CRATE" --locked --color=never
fi

step "version bump"
snapshot_tree
bump_workspace_version
run "cargo check after the bump" cargo check --workspace --all-targets --all-features --color=never
run "cargo check --locked" cargo check --workspace --all-targets --all-features --locked --color=never
say SUCCESS "$LOCK_FILE records $VERSION"

step "changelog"
move_changelog_section
if [[ "$MODE" == "release" ]]; then
	refuse_commit_on_protected_branch
fi

step "release-profile rebuild"
if [[ "$MODE" == "dry-run" ]]; then
	run "cargo build --release" cargo build --release --locked --color=never
	DRY_RUN_PRINTED="$("${CARGO_TARGET_DIR:-$REPO/target}/release/$BIN_CRATE" --version 2>&1 || true)"
	[[ "$DRY_RUN_PRINTED" == "$BIN_CRATE $VERSION ("* ]] ||
		die "the release build prints '$DRY_RUN_PRINTED', expected '$BIN_CRATE $VERSION (commit ..., built ...)'"
	say SUCCESS "the release build runs and reports $VERSION; the dry run installs nothing"
else
	run "./tools/reinstall.sh --skip-gate" ./tools/reinstall.sh --skip-gate
fi

if [[ "$MODE" == "dry-run" ]]; then
	step "dist rehearsal"
	REHEARSAL_TARGET=""
	REHEARSAL_TARGETS=()
	while IFS= read -r REHEARSAL_TARGET; do
		[[ -n "$REHEARSAL_TARGET" ]] && REHEARSAL_TARGETS+=("$REHEARSAL_TARGET")
	done < <(dist_targets)
	if [[ ${#REHEARSAL_TARGETS[@]} -gt 0 ]]; then
		say SUCCESS "dist plan recognizes ${#REHEARSAL_TARGETS[@]} targets: ${REHEARSAL_TARGETS[*]}"
	else
		say WARNING "dist plan returned no targets for v$VERSION"
	fi
	if REHEARSAL_PROBLEM="$(changelog_section_problem)"; then
		say WARNING "$CHANGELOG: $REHEARSAL_PROBLEM; the GitHub Release step would refuse to publish"
	else
		say SUCCESS "$CHANGELOG has one non-empty [$VERSION] section"
	fi
	if gh release view "v$VERSION" >/dev/null 2>&1; then
		say WARNING "a GitHub release v$VERSION already exists on this repository"
	else
		say SUCCESS "no GitHub release v$VERSION exists yet on this repository"
	fi
	if gh release view "v$VERSION" --repo "$PUBLIC_RELEASE_REPO" >/dev/null 2>&1; then
		say WARNING "a GitHub release v$VERSION already exists on $PUBLIC_RELEASE_REPO"
	else
		say SUCCESS "no GitHub release v$VERSION exists yet on $PUBLIC_RELEASE_REPO"
	fi

	restore_tree
	say INFO "dry run: the bump was written to disk, built without installing, and then reverted"
	say INFO "$ROOT_MANIFEST, $CLI_MANIFEST, $LOCK_FILE, and $CHANGELOG are back on $CURRENT_VERSION"
	say SUCCESS "dry run finished; nothing was published, tagged, or pushed"
	exit 0
fi

step "confirmation"
say INFO "this will publish and push:"
say INFO "  $LIB_CRATE $VERSION to crates.io"
say INFO "  $BIN_CRATE $VERSION to crates.io, after the library is live"
say INFO "  prebuilt binaries for whichever of the eight targets this machine can build, with the SBOM, the attribution file, and a minisign signature over the checksums"
if branch_is_protected "$BRANCH"; then
	say INFO "  no commit: $BRANCH already carries the $VERSION bump"
else
	say INFO "  a commit on $BRANCH carrying the bump and the changelog"
fi
say INFO "  tag v$VERSION ($(tag_flag)) pushed to $REMOTE"
say INFO "  a GitHub release v$VERSION carrying the built binaries, here and on $PUBLIC_RELEASE_REPO"
say INFO "  the Homebrew formula pushed to ${OWNPG_HOMEBREW_TAP:-devops-infinity/homebrew-tap} and the npm package published, unless OWNPG_SKIP_TAP or OWNPG_SKIP_NPM is 1"
say WARNING "a published crates.io version can never be replaced, only yanked"
confirm "$VERSION"

step "publish"
IRREVERSIBLE=1
if [[ $LIB_PUBLISHED -eq 1 ]]; then
	say INFO "$LIB_CRATE $VERSION is already on crates.io, skipping its upload"
else
	publish_crate "$LIB_CRATE"
	wait_for_crate "$LIB_CRATE"
fi
if [[ $BIN_PUBLISHED -eq 1 ]]; then
	say INFO "$BIN_CRATE $VERSION is already on crates.io, skipping its upload"
else
	run "packaging proof for $BIN_CRATE" cargo publish --dry-run --locked --allow-dirty -p "$BIN_CRATE"
	publish_crate "$BIN_CRATE"
	wait_for_crate "$BIN_CRATE"
fi

step "commit, tag, push"
tracked_release_files | xargs git add -- || die "git add failed"
if git diff --cached --quiet; then
	say INFO "nothing to commit; $BRANCH already carries the $VERSION bump"
else
	git commit -m "chore: release $VERSION" >/dev/null || die "git commit failed"
	say SUCCESS "committed the bump"
fi
if git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null; then
	say WARNING "tag v$VERSION already exists, leaving it alone"
else
	git tag "$(tag_flag)" "v$VERSION" -m "OwnPG v$VERSION" || die "tagging v$VERSION failed"
	say SUCCESS "tagged v$VERSION"
fi
MUTATED=0
if [[ "$(git rev-parse HEAD)" == "$(git rev-parse "$REMOTE/$BRANCH")" ]]; then
	say INFO "$BRANCH is level with $REMOTE/$BRANCH, so only the tag is pushed"
else
	git push "$REMOTE" "$BRANCH" ||
		die "pushing $BRANCH failed; finish with: git push $REMOTE $BRANCH && git push $REMOTE v$VERSION"
fi
git push "$REMOTE" "v$VERSION" || die "pushing the tag failed; finish with: git push $REMOTE v$VERSION"
PUSHED=1
say SUCCESS "pushed v$VERSION"

step "dist build"
build_dist_artifacts

step "signature"
sign_checksums

step "registry manifest"
write_registry_manifest
say INFO "publish to the MCP Registry by hand: cd target/distrib, run mcp-publisher login github --token with a classic PAT that has only read:org, then mcp-publisher publish; the browser login grants only your personal namespace"

step "GitHub Release"
publish_github_release ""
publish_github_release "$PUBLIC_RELEASE_REPO"
PUSHED=0

step "Homebrew tap"
publish_homebrew_formula

step "npm package"
publish_npm_package

say SUCCESS "released $VERSION"
