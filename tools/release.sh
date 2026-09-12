#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
SELF="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/$(basename -- "${BASH_SOURCE[0]}")"
LIB_CRATE="ownpg-core"
BIN_CRATE="ownpg"
ROOT_MANIFEST="Cargo.toml"
CLI_MANIFEST="crates/ownpg/Cargo.toml"
LOCK_FILE="Cargo.lock"
CHANGELOG="CHANGELOG.md"
ATTRIBUTION="THIRD-PARTY.txt"
BRANCH="main"
REMOTE="origin"
PUBLIC_RELEASE_REPO="devops-infinity/ownpg-releases"
RELEASE_URL_BASE="https://github.com/devops-infinity/ownpg-releases/releases/tag"
REGISTRY_API="https://crates.io/api/v1/crates"
USER_AGENT="ownpg-release-script (+https://github.com/devops-infinity/ownpg-releases)"
MINISIGN_KEY="${OWNPG_MINISIGN_KEY:-$HOME/.minisign/minisign.key}"
AUDIT_EXEMPT=" release.sh "
PROPAGATE_TRIES=30
PROPAGATE_WAIT=10

MODE="release"
VERSION=""
BUMP=""
CURRENT_VERSION=""
ASSUME_YES=0
STEP="startup"
STEP_NO=0
WORK_DIR=""
MUTATED=0
IRREVERSIBLE=0
UPLOADED=0
RESUMING=0
PUSHED=0
CRATE_HTTP_STATUS=""
CRATE_BODY_FILE=""

usage() {
	cat <<'USAGE'
Usage: tools/release.sh --patch | --minor | --major [options]
       tools/release.sh --version X.Y.Z [options]
       tools/release.sh --dry-run --patch
       tools/release.sh --yank X.Y.Z | --unyank X.Y.Z

  --major          release the next major version, worked out from the current one
  --minor          release the next minor version
  --patch          release the next patch version
  --version X.Y.Z  release this exact version instead of a computed one
  --dry-run        run every check and the bump, publish nothing, revert the bump
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

Environment:
  OWNPG_MINISIGN_KEY        the minisign secret key (default ~/.minisign/minisign.key)
  OWNPG_CODESIGN_IDENTITY   Developer ID Application identity for the macOS binaries
  OWNPG_NOTARY_PROFILE      notarytool keychain profile; unset skips signing with a warning
  OWNPG_WINDOWS_SIGN_CERT   PKCS#12 certificate for the Windows binaries, signed with osslsigncode
  OWNPG_WINDOWS_SIGN_PASS   the password of that certificate; unset skips signing with a warning
  OWNPG_HOMEBREW_TAP        the tap repository (default devops-infinity/homebrew-tap)
  OWNPG_SKIP_TAP            set to 1 to leave the formula in target/distrib instead of pushing it
  OWNPG_SKIP_NPM            set to 1 to leave the npm package in target/distrib instead of publishing it
USAGE
}

say() { printf '[%s] %s\n' "$1" "$2"; }
die() {
	say FAILED "$1"
	exit 1
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
	say INFO "finish by hand with:"
	printf '  git add -- %s %s %s %s\n' "$ROOT_MANIFEST" "$CLI_MANIFEST" "$LOCK_FILE" "$CHANGELOG"
	printf '  git commit -m "chore: release %s"\n' "$VERSION"
	printf '  git tag %s v%s -m "OwnPG v%s"\n' "$(tag_flag)" "$VERSION" "$VERSION"
	printf '  git push %s %s\n' "$REMOTE" "$BRANCH"
	printf '  git push %s v%s\n' "$REMOTE" "$VERSION"
	print_manual_binary_steps
}

cleanup() {
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
		rm -rf -- "$WORK_DIR"
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
		tail -n 40 "$log"
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

require_gh_auth() {
	gh auth status >/dev/null 2>&1 || die "gh is not authenticated: run 'gh auth login'"
}

require_minisign_key() {
	[[ -f "$MINISIGN_KEY" ]] || die "no minisign secret key at $MINISIGN_KEY; run 'minisign -G' once, or point OWNPG_MINISIGN_KEY at the key"
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
	CRATE_HTTP_STATUS="$(curl -sS -A "$USER_AGENT" --max-time 30 -o "$CRATE_BODY_FILE" \
		-w '%{http_code}' "$REGISTRY_API/$name" 2>/dev/null || printf '000')"
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
	*)
		die "crates.io answered $CRATE_HTTP_STATUS for $name"
		;;
	esac
}

check_registry_state() {
	local lib_published=0 bin_published=0
	if published_already "$LIB_CRATE"; then lib_published=1; fi
	if published_already "$BIN_CRATE"; then bin_published=1; fi

	if [[ $lib_published -eq 1 && $bin_published -eq 1 ]]; then
		die "both crates are already on crates.io at $VERSION; a published version cannot be replaced"
	fi
	if [[ $bin_published -eq 1 ]]; then
		die "$BIN_CRATE $VERSION is on crates.io but $LIB_CRATE $VERSION is not; sort that out by hand"
	fi
	if [[ $lib_published -eq 1 ]]; then
		RESUMING=1
		say WARNING "$LIB_CRATE $VERSION is already on crates.io; this run resumes a part-finished release"
		return 0
	fi
	say SUCCESS "$VERSION is free on crates.io for both crates"
}

audit_files() {
	git ls-files -- crates tools ':(top,glob)*.md' | sort -u
}

collect_scannable_files() {
	local -n into="$1"
	local file
	while IFS= read -r file; do
		[[ -f "$file" ]] || continue
		[[ "$REPO/$file" != "$SELF" ]] || continue
		grep -Iq . "$file" || continue
		into+=("$file")
	done < <(audit_files)
}

audit_scan() {
	local -a files=()
	collect_scannable_files files
	[[ ${#files[@]} -gt 0 ]] || die "the house-rule audit found no files to scan"

	awk -v exempt="$AUDIT_EXEMPT" '
		function leaf(path,   parts, n) {
			n = split(path, parts, "/")
			return parts[n]
		}
		FNR == 1 { states_rule = index(exempt, " " leaf(FILENAME) " ") > 0 }
		{
			probe = tolower($0)
			found = ""
			if (!states_rule) {
				if (probe ~ /legacy/) found = "legacy"
				else if (probe ~ /backward.compat/) found = "backward compat"
				else if (probe ~ /inspired by/) found = "inspired by"
				else if (probe ~ /based on/) found = "based on"
				else if (probe ~ /ported from/) found = "ported from"
				else if (probe ~ /fork of/) found = "fork of"
			}
			if (found == "") next
			text = $0
			sub(/^[ \t]+/, "", text)
			if (length(text) > 100) text = substr(text, 1, 100) "..."
			printf "  %s:%d  %s  |  %s\n", FILENAME, FNR, found, text
		}
	' "${files[@]}"
}

house_rule_audit() {
	say INFO "auditing every tracked file under crates/ and tools/ plus the markdown files at the root"
	say INFO "these carry the rule text itself and are exempt from it:$AUDIT_EXEMPT"
	local hits
	hits="$(audit_scan)"
	if [[ -n "$hits" ]]; then
		printf '%s\n' "$hits"
		die "the house-rule audit refuses this release; clear every line above first"
	fi
	say SUCCESS "nothing borrowed"
	em_dash_audit
}

em_dash_audit() {
	local em_dash
	em_dash="$(printf '\342\200\224')"
	local -a files=()
	collect_scannable_files files
	[[ ${#files[@]} -gt 0 ]] || die "the em dash audit found no files to scan"

	local hits
	hits="$(grep -rn -- "$em_dash" "${files[@]}" 2>/dev/null || true)"
	if [[ -n "$hits" ]]; then
		printf '%s\n' "$hits"
		die "an em dash reached the release artifacts; clear every line above first"
	fi
	say SUCCESS "zero em dashes"
}

deny_reasons() {
	local code="$1" names=""
	if ((code & 1)); then names="$names advisories"; fi
	if ((code & 2)); then names="$names bans"; fi
	if ((code & 4)); then names="$names licenses"; fi
	if ((code & 8)); then names="$names sources"; fi
	if [[ -z "$names" ]]; then
		printf 'unrecognized exit %s' "$code"
	else
		printf '%s (exit %s)' "${names# }" "$code"
	fi
}

check_deny() {
	STEP="cargo deny check"
	local log code=0
	log="$WORK_DIR/cargo-deny.log"
	cargo deny check >"$log" 2>&1 || code=$?
	if [[ $code -ne 0 ]]; then
		say INFO "last 40 lines of output:"
		tail -n 40 "$log"
		die "cargo deny check failed: $(deny_reasons "$code")"
	fi
	say SUCCESS "cargo deny check"
}

write_attribution() {
	run "cargo about generate" cargo about generate about.hbs -o "$ATTRIBUTION"
	[[ -s "$ATTRIBUTION" ]] || die "$ATTRIBUTION came out empty"
}

snapshot_tree() {
	local file
	for file in "$ROOT_MANIFEST" "$CLI_MANIFEST" "$LOCK_FILE" "$CHANGELOG"; do
		mkdir -p -- "$WORK_DIR/backup/$(dirname -- "$file")"
		cp -p -- "$file" "$WORK_DIR/backup/$file"
	done
	MUTATED=1
}

restore_tree() {
	local file
	for file in "$ROOT_MANIFEST" "$CLI_MANIFEST" "$LOCK_FILE" "$CHANGELOG"; do
		if [[ -f "$WORK_DIR/backup/$file" ]]; then
			cp -p -- "$WORK_DIR/backup/$file" "$file"
		fi
	done
	MUTATED=0
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
	cat -- "$temp" >"$target"
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
}

move_changelog_section() {
	local today program="$WORK_DIR/changelog.awk"
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
		tail -n 40 "$log"
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
		tail -n 40 "$log"
		die "dist plan failed (exit $code) for v$VERSION"
	fi
	jq -er '.ci.github.artifacts_matrix.include[].targets[]' "$plan" ||
		die "dist plan produced JSON without .ci.github.artifacts_matrix.include[].targets[]; the dist output schema may have changed"
}

c_flags_for() {
	case "$1" in
	aarch64-pc-windows-msvc) printf -- '-Wno-error=incompatible-pointer-types -D_mm_pause=__builtin_arm_yield' ;;
	*-pc-windows-msvc) printf -- '-Wno-error=incompatible-pointer-types' ;;
	*) printf '' ;;
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
		if CFLAGS="$(c_flags_for "$target")" dist build --tag="v$VERSION" --artifacts=local --target="$target" --no-local-paths \
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
			tail -n 40 "$log"
			die "$target failed to build and it does not look like a missing cross toolchain; this may be a real defect, not something safe to skip"
		fi
	done
	[[ ${#built[@]} -gt 0 ]] || die "no target built; nothing to release"
	if [[ ${#skipped[@]} -gt 0 ]]; then
		say WARNING "skipped target(s), releasing the rest: ${skipped[*]}"
		say WARNING "the shell and PowerShell installers still offer every configured target, so"
		say WARNING "a user on a skipped target gets a failed download, not a clear message"
		if [[ $ASSUME_YES -ne 1 ]]; then
			[[ -t 0 ]] || die "no terminal to confirm a partial target set on; pass --yes for an unattended run"
			local typed=""
			printf 'built %s of %s targets (%s); type %s to publish this partial set, anything else stops here (crates.io and the tag are already live either way): ' \
				"${#built[@]}" "${#targets[@]}" "${built[*]}" "$VERSION"
			IFS= read -r typed || true
			[[ "$typed" == "$VERSION" ]] ||
				die "that did not match $VERSION; crates.io and the tag are already live, finish the binaries by hand when ready"
		fi
	fi
	sign_macos_binaries "${built[@]}"
	sign_windows_binaries "${built[@]}"
	build_mcpb_bundles "${built[@]}"
	run "dist build --artifacts=global" dist build --tag="v$VERSION" --artifacts=global --no-local-paths
	cp -- "$ATTRIBUTION" target/distrib/ || die "could not place $ATTRIBUTION next to the archives"
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
		run "osslsigncode ($target)" osslsigncode sign -pkcs12 "$cert" -pass "$pass" -n "OwnPG" -i "$RELEASE_URL_BASE" -t http://timestamp.digicert.com -in "$exe" -out "$exe.signed"
		mv -f -- "$exe.signed" "$exe" || die "could not replace $exe with the signed binary"
		run "osslsigncode verify ($target)" osslsigncode verify -in "$exe"
		rm -f -- "$archive"
		(cd "$stage" && zip -q -r "$REPO/$archive" .) || die "could not repack $archive after signing"
		say SUCCESS "signed $target"
	done
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
	require_tools npm
	npm whoami >/dev/null 2>&1 || die "npm is not logged in: run 'npm login' first"
	run "npm publish" npm publish "$package" --access public
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

write_server_json() {
	local bundle name sha packages
	packages="$(jq -c --arg version "$VERSION" '.packages | map(select(.registryType == "cargo") | .version = $version)' server.json)"
	shopt -s nullglob
	for bundle in target/distrib/*.mcpb; do
		name="$(basename "$bundle")"
		sha="$(shasum -a 256 "$bundle" | awk '{print $1}')"
		packages="$(jq -c --arg version "$VERSION" --arg url "https://github.com/$PUBLIC_RELEASE_REPO/releases/download/v$VERSION/$name" --arg sha "$sha" \
			'. + [{registryType: "mcpb", registryBaseUrl: "https://github.com", identifier: $url, version: $version, fileSha256: $sha, transport: {type: "stdio"}}]' <<<"$packages")"
	done
	shopt -u nullglob
	jq --arg version "$VERSION" --argjson packages "$packages" '.version = $version | .packages = $packages' server.json >"$WORK_DIR/server.json" || die "could not rewrite server.json"
	mv -- "$WORK_DIR/server.json" server.json
	say SUCCESS "server.json carries $VERSION and $(jq '.packages | length' server.json) package entries"
}

sign_checksums() {
	local sums="target/distrib/sha256.sum"
	[[ -f "$sums" ]] || die "$sums is missing; the global build did not produce a checksum file"
	run "minisign -Sm $sums" minisign -Sm "$sums" -s "$MINISIGN_KEY" -t "OwnPG $VERSION"
	[[ -f "$sums.minisig" ]] || die "minisign did not write $sums.minisig"
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
	if problem="$(changelog_section_problem)"; then
		die "$CHANGELOG: $problem; fix it by hand before the release notes can be trusted"
	fi
	changelog_section >"$notes"

	local view="$WORK_DIR/gh-view-${target_repo//\//-}.json" view_err="$WORK_DIR/gh-view-${target_repo//\//-}.err"
	if gh release view "v$VERSION" "${repo_flag[@]}" --json assets >"$view" 2>"$view_err"; then
		local name
		local -a have_names=() want_names=()
		while IFS= read -r name; do
			[[ -n "$name" ]] && have_names+=("$name")
		done < <(jq -r '.assets[].name' "$view" | sort)
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
		die "refusing to leave a mismatched release in place on $where; settle it with 'gh release upload v$VERSION \$(find target/distrib -maxdepth 1 -type f) ${repo_flag[*]} --clobber' or delete v$VERSION there and rerun"
	elif ! grep -qi 'release not found\|not found' "$view_err"; then
		say INFO "last 40 lines of output:"
		tail -n 40 "$view_err"
		die "could not ask $where whether v$VERSION already exists; not creating a release blind"
	fi

	local label="gh release create v$VERSION on $where"
	local log
	log="$(log_path_for "$label")"
	run "$label" gh release create "v$VERSION" "${assets[@]}" "${repo_flag[@]}" \
		--title "OwnPG $VERSION" --notes-file "$notes"
	local release_url
	release_url="$(grep -oE 'https://github\.com/[^[:space:]]+/releases/tag/[^[:space:]]+' "$log" | tail -n 1)"
	[[ -z "$release_url" ]] || say SUCCESS "release page: $release_url"
}

wait_for_crate() {
	local name="$1" tries=0
	say INFO "waiting for $name $VERSION to show up on crates.io"
	while [[ $tries -lt $PROPAGATE_TRIES ]]; do
		fetch_crate "$name"
		if [[ "$CRATE_HTTP_STATUS" == "200" ]] && crate_has_version "$VERSION"; then
			say SUCCESS "$name $VERSION is live"
			return 0
		fi
		tries=$((tries + 1))
		sleep "$PROPAGATE_WAIT"
	done
	die "$name $VERSION did not appear after $((PROPAGATE_TRIES * PROPAGATE_WAIT))s; publish $BIN_CRATE by hand once it does"
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
			tail -n 40 "$WORK_DIR/$action-$name.log"
			die "$action of $name $VERSION failed (exit $code)"
		fi
		say SUCCESS "$action $name $VERSION"
	done
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

if [[ -n "$BUMP" && "$MODE" != "release" && "$MODE" != "dry-run" ]]; then
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

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ownpg-release.XXXXXX")"

say INFO "repository $REPO"

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
[[ -z "$(git status --porcelain)" ]] || die "the working tree is not clean; commit or stash first"
say SUCCESS "working tree is clean"
CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$CURRENT_BRANCH" == "$BRANCH" ]] || die "on branch $CURRENT_BRANCH; a release is cut from $BRANCH"
say SUCCESS "on $BRANCH"
git fetch --quiet "$REMOTE" "$BRANCH" || die "could not fetch $REMOTE/$BRANCH"
BEHIND="$(git rev-list --count "HEAD..$REMOTE/$BRANCH")"
[[ "$BEHIND" -eq 0 ]] || die "$BRANCH is $BEHIND commit(s) behind $REMOTE/$BRANCH; pull first"
say SUCCESS "$BRANCH is level with $REMOTE/$BRANCH"
require_credentials
require_gh_auth
require_minisign_key
check_dist_version
CURRENT_VERSION="$(read_current_version)"
[[ -n "$CURRENT_VERSION" ]] || die "could not read the version from $ROOT_MANIFEST"
valid_version "$CURRENT_VERSION" || die "the version in $ROOT_MANIFEST is not a semver triple: $CURRENT_VERSION"
if [[ -n "$BUMP" ]]; then
	VERSION="$(next_version "$CURRENT_VERSION" "$BUMP")" || die "unknown bump: $BUMP"
	say INFO "$BUMP release: $CURRENT_VERSION -> $VERSION"
fi
check_registry_state
if [[ "$VERSION" == "$CURRENT_VERSION" ]]; then
	[[ $RESUMING -eq 1 ]] || die "$VERSION is not greater than the current $CURRENT_VERSION"
	say WARNING "the manifests already carry $VERSION, so the bump step will change nothing"
else
	version_greater "$VERSION" "$CURRENT_VERSION" || die "$VERSION is not greater than the current $CURRENT_VERSION"
	say SUCCESS "$CURRENT_VERSION -> $VERSION"
fi

step "house-rule audit"
house_rule_audit

step "verification gate"
run "cargo fmt --all -- --check" cargo fmt --all -- --check
run "cargo check" cargo check --workspace --all-targets --all-features --locked --color=never
run "cargo clippy" cargo clippy --workspace --all-targets --all-features --locked --color=never -- -D warnings
run "cargo nextest" cargo nextest run --workspace --all-features --locked --no-tests=warn --color=never
run "cargo test --doc" cargo test --workspace --all-features --locked --doc
run "cargo doc" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
run "cargo audit" cargo audit --deny warnings
check_deny
run "cargo machete" cargo machete
check_semver
write_attribution

step "packaging proof"
run "cargo publish --dry-run -p $LIB_CRATE" cargo publish --dry-run -p "$LIB_CRATE" --locked --color=never

step "version bump"
snapshot_tree
bump_workspace_version
run "cargo check after the bump" cargo check --workspace --all-targets --all-features --color=never
run "cargo check --locked" cargo check --workspace --all-targets --all-features --locked --color=never
say SUCCESS "$LOCK_FILE records $VERSION"

step "changelog"
move_changelog_section

step "release-profile rebuild"
run "./tools/reinstall.sh" ./tools/reinstall.sh

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
	say INFO "dry run: the bump was written to disk, built, and then reverted"
	say INFO "$ROOT_MANIFEST, $CLI_MANIFEST, $LOCK_FILE, and $CHANGELOG are back on $CURRENT_VERSION"
	say SUCCESS "dry run finished; nothing was published, tagged, or pushed"
	exit 0
fi

step "confirmation"
say INFO "this will publish and push:"
say INFO "  $LIB_CRATE $VERSION to crates.io"
say INFO "  $BIN_CRATE $VERSION to crates.io, after the library is live"
say INFO "  prebuilt binaries for whichever of the eight targets this machine can build, with the SBOM, the attribution file, and a minisign signature over the checksums"
say INFO "  a commit on $BRANCH carrying the bump and the changelog"
say INFO "  tag v$VERSION ($(tag_flag)) pushed to $REMOTE"
say INFO "  a GitHub release v$VERSION carrying the built binaries, here and on $PUBLIC_RELEASE_REPO"
say INFO "  the Homebrew formula pushed to ${OWNPG_HOMEBREW_TAP:-devops-infinity/homebrew-tap} and the npm package published, unless OWNPG_SKIP_TAP or OWNPG_SKIP_NPM is 1"
say WARNING "a published crates.io version can never be replaced, only yanked"
confirm "$VERSION"

step "publish"
IRREVERSIBLE=1
publish_crate "$LIB_CRATE"
wait_for_crate "$LIB_CRATE"
run "packaging proof for $BIN_CRATE" cargo publish --dry-run --locked --allow-dirty -p "$BIN_CRATE"
publish_crate "$BIN_CRATE"
wait_for_crate "$BIN_CRATE"

step "commit, tag, push"
git add -- "$ROOT_MANIFEST" "$CLI_MANIFEST" "$LOCK_FILE" "$CHANGELOG" || die "git add failed"
if git diff --cached --quiet; then
	say WARNING "nothing staged; the bump is already committed"
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
git push "$REMOTE" "$BRANCH" ||
	die "pushing $BRANCH failed; finish with: git push $REMOTE $BRANCH && git push $REMOTE v$VERSION"
git push "$REMOTE" "v$VERSION" || die "pushing the tag failed; finish with: git push $REMOTE v$VERSION"
PUSHED=1
say SUCCESS "pushed v$VERSION"

step "dist build"
build_dist_artifacts

step "signature"
sign_checksums

step "registry manifest"
write_server_json
say INFO "publish to the MCP Registry by hand once the release is up: mcp-publisher login github && mcp-publisher publish"

step "GitHub Release"
publish_github_release ""
publish_github_release "$PUBLIC_RELEASE_REPO"
PUSHED=0

step "Homebrew tap"
publish_homebrew_formula

step "npm package"
publish_npm_package

say SUCCESS "released $VERSION"
