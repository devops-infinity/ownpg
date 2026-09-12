#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="ownpg"
PROFILE="release"
INSTALL_DIR="${OWNPG_INSTALL_DIR:-$HOME/.local/bin}"
SKIP_GATE=0
KEEP_CACHE=0
FULL_CLEAN=0
KEEP_BUILD=0
STALE_DAYS="${OWNPG_STALE_DAYS:-7}"
KEEP_INCREMENTAL=8
BUILD_DIR=""
STALE_HASHES=""

usage() {
	cat <<'USAGE'
Usage: tools/reinstall.sh [options]

  --debug           build the debug profile instead of release
  --dir <path>      install into this directory (default ~/.local/bin)
  --skip-gate       install without running the verification gate first
  --keep-cache      leave the OwnPG cache directory in place
  --full-clean      empty target/ first, so the next build starts cold
  --keep-build      leave target/ alone this run
  --stale-days <n>  drop unused build data older than this (default 7)
  -h, --help        show this

Every run drops stale and orphaned build data from target/ and says how much
it reclaimed. The caches the next build reuses are kept, so a rebuild stays
fast. Use --full-clean to reclaim all of it and pay for one cold build.

Environment:
  OWNPG_INSTALL_DIR   same as --dir
  OWNPG_STALE_DAYS    same as --stale-days
USAGE
}

say() { printf '[%s] %s\n' "$1" "$2"; }
die() {
	say FAILED "$1"
	exit 1
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--debug)
		PROFILE="debug"
		shift
		;;
	--dir)
		[[ $# -ge 2 ]] || die "--dir needs a path"
		INSTALL_DIR="$2"
		shift 2
		;;
	--skip-gate)
		SKIP_GATE=1
		shift
		;;
	--keep-cache)
		KEEP_CACHE=1
		shift
		;;
	--full-clean)
		FULL_CLEAN=1
		shift
		;;
	--keep-build)
		KEEP_BUILD=1
		shift
		;;
	--stale-days)
		[[ $# -ge 2 ]] || die "--stale-days needs a number of days"
		STALE_DAYS="$2"
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

[[ "$STALE_DAYS" =~ ^[0-9]+$ ]] || die "stale days must be a whole number, got: $STALE_DAYS"

cd "$REPO"

command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH"

say INFO "repository $REPO"
say INFO "profile $PROFILE, installing into $INSTALL_DIR"

REMOVED=0
for CANDIDATE in "$INSTALL_DIR/$BIN" /usr/local/bin/"$BIN" "$HOME/.cargo/bin/$BIN"; do
	if [[ -e "$CANDIDATE" ]]; then
		rm -f "$CANDIDATE" && say INFO "removed $CANDIDATE"
		REMOVED=$((REMOVED + 1))
	fi
done
if command -v cargo-uninstall >/dev/null 2>&1 || cargo uninstall --help >/dev/null 2>&1; then
	cargo uninstall "$BIN" >/dev/null 2>&1 && say INFO "removed the cargo-installed copy" || true
fi
[[ $REMOVED -eq 0 ]] && say INFO "no previously installed copy found"

if [[ $KEEP_CACHE -eq 0 ]]; then
	for CACHE in \
		"${XDG_CACHE_HOME:-$HOME/.cache}/ownpg" \
		"${LOCALAPPDATA:-$HOME/AppData/Local}/devops/ownpg/cache"; do
		if [[ -d "$CACHE" ]]; then
			rm -rf "$CACHE" && say INFO "cleared cache $CACHE"
		fi
	done
fi

if [[ "$(stat -f '%m' . 2>/dev/null)" =~ ^[0-9]+$ ]]; then
	mtimes() { stat -f '%Fm %N' "$@"; }
elif [[ "$(stat -c '%.9Y' . 2>/dev/null)" =~ ^[0-9] ]]; then
	mtimes() { stat -c '%.9Y %n' "$@"; }
else
	mtimes() { stat -c '%Y %n' "$@"; }
fi

human_kb() {
	awk -v kb="${1:-0}" 'BEGIN {
		split("KB MB GB TB", unit, " ")
		i = 1
		while (kb >= 1024 && i < 4) {
			kb /= 1024
			i++
		}
		if (i == 1) {
			printf "%d %s\n", kb, unit[i]
		} else {
			printf "%.1f %s\n", kb, unit[i]
		}
	}'
}

dir_kb() {
	if [[ ! -d "$1" ]]; then
		printf '0\n'
		return 0
	fi
	du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 + 0 }'
}

resolve_build_dir() {
	local target="$REPO/target" repo_real
	if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
		say INFO "CARGO_TARGET_DIR is set, so target/ is left alone"
		return 1
	fi
	[[ -d "$target" ]] || return 1
	if [[ -L "$target" ]]; then
		say WARNING "target is a link, so it is left alone"
		return 1
	fi
	repo_real="$(cd -- "$REPO" && pwd -P)" || return 1
	BUILD_DIR="$(cd -- "$target" && pwd -P)" || return 1
	case "$BUILD_DIR" in
	"$repo_real"/*) return 0 ;;
	esac
	say WARNING "the build directory sits outside $REPO, so it is left alone"
	BUILD_DIR=""
	return 1
}

inside_build_dir() {
	local path=$1 real
	[[ -n "$BUILD_DIR" && "$path" == /* ]] || return 1
	[[ -d "$path" && ! -L "$path" ]] || return 1
	real="$(cd -- "$path" 2>/dev/null && pwd -P)" || return 1
	case "$real" in
	"$BUILD_DIR" | "$BUILD_DIR"/*) return 0 ;;
	esac
	return 1
}

drop_path() {
	local path=$1
	[[ -n "$path" ]] || return 0
	if [[ "$path" != /* || "$path" == *..* ]]; then
		say WARNING "refused to remove $path"
		return 0
	fi
	[[ -e "$path" || -L "$path" ]] || return 0
	if ! inside_build_dir "$(dirname -- "$path")"; then
		say WARNING "refused to remove $path"
		return 0
	fi
	rm -rf -- "$path" || say WARNING "could not remove $path"
}

newest_mtime() {
	local dir=$1 newest=0 stamp line
	while IFS= read -r line; do
		stamp="${line%% *}"
		stamp="${stamp%%.*}"
		[[ "$stamp" =~ ^[0-9]+$ ]] || continue
		if [[ $stamp -gt $newest ]]; then
			newest=$stamp
		fi
	done < <(mtimes "$dir" "$dir"/* 2>/dev/null)
	printf '%s\n' "$newest"
}

add_stale_hashes() {
	[[ -n "$1" ]] || return 0
	STALE_HASHES="$(printf '%s\n%s\n' "$STALE_HASHES" "$1")"
}

prune_stale_dirs() {
	local active="$BUILD_DIR/$PROFILE" cutoff entry touched
	cutoff=$(($(date +%s) - STALE_DAYS * 86400))
	while IFS= read -r entry; do
		if [[ "$entry" == "$active" ]]; then
			continue
		fi
		if [[ "$entry" == "$BUILD_DIR/tmp" ]]; then
			drop_path "$entry"
			continue
		fi
		touched="$(newest_mtime "$entry")"
		if [[ $touched -gt 0 && $touched -lt $cutoff ]]; then
			say INFO "dropping ${entry#"$BUILD_DIR"/}; nothing has touched it for $STALE_DAYS days"
			drop_path "$entry"
		fi
	done < <(find "$BUILD_DIR" -mindepth 1 -maxdepth 1 -type d 2>/dev/null)
}

prune_units() {
	local dir=$1
	local fingerprints="$dir/.fingerprint"
	local live stamped current stale found newest rustc_id hash entry
	[[ -d "$fingerprints" ]] || return 0

	live="$(find "$fingerprints" -mindepth 1 -maxdepth 1 -type d 2>/dev/null |
		sed -n 's|.*-\([0-9a-f]\{16\}\)$|\1|p' | sort -u)" || true
	[[ -n "$live" ]] || return 0

	STALE_HASHES=""
	newest="$(mtimes "$fingerprints"/*/*.json 2>/dev/null | sort -rn | head -1)" || true
	rustc_id=""
	if [[ -n "$newest" ]]; then
		rustc_id="$(sed -n 's|.*"rustc":\([0-9]*\).*|\1|p' "${newest#* }" 2>/dev/null | head -1)" || true
	fi
	if [[ -n "$rustc_id" ]]; then
		stamped="$(grep -l -F '"rustc":' "$fingerprints"/*/*.json 2>/dev/null |
			sed -n 's|.*-\([0-9a-f]\{16\}\)/[^/]*$|\1|p' | sort -u)" || true
		current="$(grep -l -F "\"rustc\":$rustc_id," "$fingerprints"/*/*.json 2>/dev/null |
			sed -n 's|.*-\([0-9a-f]\{16\}\)/[^/]*$|\1|p' | sort -u)" || true
		if [[ -n "$stamped" && -n "$current" ]]; then
			stale="$(comm -23 <(printf '%s\n' "$stamped") <(printf '%s\n' "$current"))" || true
			if [[ -n "$stale" ]]; then
				say INFO "dropping build data left by an older toolchain"
			fi
			add_stale_hashes "$stale"
		fi
	fi

	if [[ -f "$REPO/Cargo.lock" && -d "$dir/deps" ]]; then
		found="$(grep -o -H -E 'registry/src/[^/ ]+/[^/ ]+/' "$dir"/deps/*.d 2>/dev/null |
			awk -v lock="$REPO/Cargo.lock" '
				BEGIN {
					name = ""
					while ((getline line < lock) > 0) {
						if (split(line, field, "\"") < 2) {
							continue
						}
						if (line ~ /^name = /) {
							name = field[2]
						} else if (line ~ /^version = / && name != "") {
							known[name "-" field[2]] = 1
							name = ""
						}
					}
				}
				{
					at = index($0, ":registry/src/")
					if (at == 0) {
						next
					}
					parts = split(substr($0, at + 1), piece, "/")
					if (!(piece[parts - 1] in known)) {
						print substr($0, 1, at - 1)
					}
				}
			' | sed -n 's|.*-\([0-9a-f]\{16\}\)\.d$|\1|p' | sort -u)" || true
		add_stale_hashes "$found"
	fi

	if [[ -d "$dir/deps" ]]; then
		found="$(find "$dir/deps" -mindepth 1 -maxdepth 1 2>/dev/null |
			sed -n 's|.*/[^/]*-\([0-9a-f]\{16\}\)\(\..*\)\{0,1\}$|\1|p' | sort -u |
			comm -23 - <(printf '%s\n' "$live"))" || true
		add_stale_hashes "$found"
	fi

	if [[ -d "$dir/build" ]]; then
		found="$(find "$dir/build" -mindepth 1 -maxdepth 1 -type d 2>/dev/null |
			sed -n 's|.*-\([0-9a-f]\{16\}\)$|\1|p' | sort -u |
			comm -23 - <(printf '%s\n' "$live"))" || true
		add_stale_hashes "$found"
	fi

	[[ -n "$STALE_HASHES" ]] || return 0
	while IFS= read -r hash; do
		[[ -n "$hash" ]] || continue
		for entry in \
			"$fingerprints"/*-"$hash" \
			"$dir"/deps/*-"$hash" \
			"$dir"/deps/*-"$hash".* \
			"$dir"/build/*-"$hash"; do
			if [[ -e "$entry" ]]; then
				drop_path "$entry"
			fi
		done
	done < <(printf '%s\n' "$STALE_HASHES" | sort -u)
	STALE_HASHES=""
}

prune_incremental() {
	local dir="$1/incremental" line entry
	local caches=()
	[[ -d "$dir" ]] || return 0
	while IFS= read -r line; do
		caches+=("$line")
	done < <(find "$dir" -mindepth 1 -maxdepth 1 -type d 2>/dev/null)
	[[ ${#caches[@]} -gt $KEEP_INCREMENTAL ]] || return 0
	while IFS= read -r line; do
		entry="${line#* }"
		drop_path "$entry"
	done < <(mtimes "${caches[@]}" 2>/dev/null | sort -rn | tail -n +$((KEEP_INCREMENTAL + 1)))
}

prune_objects() {
	local deps="$1/deps"
	inside_build_dir "$deps" || return 0
	find "$deps" -maxdepth 1 -type f -name '*.rcgu.o' -delete 2>/dev/null || true
}

clean_build_dir() {
	local before after reclaimed profile

	resolve_build_dir || return 0

	before="$(dir_kb "$BUILD_DIR")"
	say INFO "target/ holds $(human_kb "$before")"

	if [[ $FULL_CLEAN -eq 1 ]]; then
		cargo clean --color=never >/dev/null 2>&1 || say WARNING "cargo clean did not finish"
	else
		if command -v cargo-sweep >/dev/null 2>&1; then
			cargo sweep --installed "$BUILD_DIR" >/dev/null 2>&1 ||
				say WARNING "cargo-sweep did not finish; the built-in clean still ran"
		fi
		prune_stale_dirs
		while IFS= read -r profile; do
			prune_units "$profile"
			prune_incremental "$profile"
			prune_objects "$profile"
		done < <(find "$BUILD_DIR" -maxdepth 3 -type d -name '.fingerprint' 2>/dev/null |
			sed 's|/\.fingerprint$||')
	fi

	after="$(dir_kb "$BUILD_DIR")"
	reclaimed=$((before - after))
	if [[ $after -eq 0 ]]; then
		say SUCCESS "cleaned $(human_kb "$before"); target/ is empty"
	elif [[ $reclaimed -gt 0 ]]; then
		say SUCCESS "cleaned $(human_kb "$reclaimed"); target/ is now $(human_kb "$after")"
	else
		say INFO "nothing stale to clean; target/ stays at $(human_kb "$after")"
	fi
}

if [[ $FULL_CLEAN -eq 1 || $KEEP_BUILD -eq 0 ]]; then
	clean_build_dir
else
	say INFO "leaving target/ alone"
fi

if [[ $SKIP_GATE -eq 0 ]]; then
	say INFO "running the verification gate"
	cargo fmt --all -- --check >/dev/null 2>&1 || die "formatting is not clean; run: cargo fmt --all"
	say SUCCESS "formatting"
	cargo check --workspace --all-targets --all-features --locked --color=never >/dev/null 2>&1 ||
		die "the check failed; run: cargo check --workspace --all-targets --all-features"
	say SUCCESS "check"
	cargo clippy --workspace --all-targets --all-features --locked --color=never -- -D warnings \
		>/dev/null 2>&1 || die "clippy found problems; run: cargo clippy --workspace --all-targets --all-features -- -D warnings"
	say SUCCESS "lint"
	if command -v cargo-nextest >/dev/null 2>&1; then
		cargo nextest run --workspace --all-features --locked --no-tests=warn --color=never >/dev/null 2>&1 ||
			die "tests failed; run: cargo nextest run --workspace --all-features"
	else
		cargo test --workspace --all-features --locked >/dev/null 2>&1 ||
			die "tests failed; run: cargo test --workspace --all-features"
	fi
	say SUCCESS "tests"
	cargo test --workspace --all-features --locked --doc >/dev/null 2>&1 ||
		die "doc tests failed; run: cargo test --workspace --all-features --doc"
	say SUCCESS "doc tests"
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked >/dev/null 2>&1 ||
		die "the docs do not build; run: RUSTDOCFLAGS=\"-D warnings\" cargo doc --workspace --all-features --no-deps"
	say SUCCESS "docs"
	for tool in cargo-audit cargo-deny cargo-machete; do
		command -v "$tool" >/dev/null 2>&1 ||
			die "$tool is not installed, so the gate cannot run; install it with: cargo install --locked $tool (or pass --skip-gate to install without the gate)"
	done
	cargo audit --deny warnings >/dev/null 2>&1 || die "cargo audit found an advisory; run: cargo audit --deny warnings"
	say SUCCESS "advisories"
	DENY_CODE=0
	cargo deny check >/dev/null 2>&1 || DENY_CODE=$?
	if [[ $DENY_CODE -ne 0 ]]; then
		DENY_NAMES=""
		((DENY_CODE & 1)) && DENY_NAMES="$DENY_NAMES advisories"
		((DENY_CODE & 2)) && DENY_NAMES="$DENY_NAMES bans"
		((DENY_CODE & 4)) && DENY_NAMES="$DENY_NAMES licenses"
		((DENY_CODE & 8)) && DENY_NAMES="$DENY_NAMES sources"
		die "cargo deny found a policy violation in:${DENY_NAMES:- an unrecognized check (exit $DENY_CODE)}; run: cargo deny check"
	fi
	say SUCCESS "dependency policy"
	cargo machete >/dev/null 2>&1 || die "cargo machete found an unused dependency; run: cargo machete"
	say SUCCESS "no unused dependency"
else
	say WARNING "skipping the verification gate"
fi

say INFO "building"
if [[ "$PROFILE" == "release" ]]; then
	cargo build --release --locked --color=never >/dev/null 2>&1 || die "the build failed; run: cargo build --release"
	BUILT="$REPO/target/release/$BIN"
else
	cargo build --locked --color=never >/dev/null 2>&1 || die "the build failed; run: cargo build"
	BUILT="$REPO/target/debug/$BIN"
fi
if [[ ! -x "$BUILT" && -x "$BUILT.exe" ]]; then
	BUILT="$BUILT.exe"
	BIN="$BIN.exe"
fi
[[ -x "$BUILT" ]] || die "expected a binary at $BUILT"
say SUCCESS "built $(du -h "$BUILT" | cut -f1 | tr -d ' ')"

mkdir -p "$INSTALL_DIR"
cp "$BUILT" "$INSTALL_DIR/$BIN" || die "could not install into $INSTALL_DIR"
chmod 755 "$INSTALL_DIR/$BIN" 2>/dev/null || true
say SUCCESS "installed $INSTALL_DIR/$BIN"

if command -v shasum >/dev/null 2>&1; then
	checksum() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
	checksum() { sha256sum "$1" | cut -d' ' -f1; }
else
	die "need shasum or sha256sum to prove the installed copy matches"
fi
BUILT_SUM="$(checksum "$BUILT")"
INSTALLED_SUM="$(checksum "$INSTALL_DIR/$BIN")"
[[ "$BUILT_SUM" == "$INSTALLED_SUM" ]] || die "the installed binary does not match the built one"
say SUCCESS "installed copy matches the build"

RESOLVED="$(command -v "$BIN" 2>/dev/null || true)"
if [[ -z "$RESOLVED" ]]; then
	say WARNING "$INSTALL_DIR is not on your PATH; add it to use '$BIN' by name"
elif [[ "$RESOLVED" != "$INSTALL_DIR/$BIN" ]]; then
	say WARNING "'$BIN' on PATH resolves to $RESOLVED, not the copy just installed"
fi

say INFO "$("$INSTALL_DIR/$BIN" --version)"
"$INSTALL_DIR/$BIN" man >/dev/null 2>&1 || die "the installed binary does not run"
say SUCCESS "ready"
