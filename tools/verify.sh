#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

say() { printf '[%s] %s\n' "$1" "$2"; }
die() {
	say FAILED "$1"
	exit 1
}

command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH"

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
		die "$tool is not installed; install it with: cargo install --locked $tool"
done
cargo audit --deny warnings >/dev/null 2>&1 || die "cargo audit found an advisory; run: cargo audit --deny warnings"
say SUCCESS "advisories"
DENY_CODE=0
cargo deny check >/dev/null 2>&1 || DENY_CODE=$?
if [[ $DENY_CODE -ne 0 ]]; then
	DENY_FAILED_CHECKS=""
	((DENY_CODE & 1)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS advisories"
	((DENY_CODE & 2)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS bans"
	((DENY_CODE & 4)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS licenses"
	((DENY_CODE & 8)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS sources"
	die "cargo deny found a policy violation in:${DENY_FAILED_CHECKS:- an unrecognized check (exit $DENY_CODE)}; run: cargo deny check"
fi
say SUCCESS "dependency policy"
cargo machete >/dev/null 2>&1 || die "cargo machete found an unused dependency; run: cargo machete"
say SUCCESS "no unused dependency"

if command -v shellcheck >/dev/null 2>&1 && command -v shfmt >/dev/null 2>&1; then
	shellcheck tools/*.sh || die "shellcheck found a problem in tools/*.sh"
	shfmt -d tools/*.sh || die "shfmt found unformatted shell in tools/*.sh; run: shfmt -w tools/*.sh"
	say SUCCESS "shell scripts"
else
	say WARNING "shellcheck or shfmt is not installed, so tools/*.sh was not checked"
fi

hits=$(git ls-files -- crates tools ':(top,glob)*.md' ':(exclude)**/LICENSE-*' |
	xargs grep -nIE 'legacy|backward.compat|inspired by|based on|ported from|fork of' 2>/dev/null |
	grep -v '^tools/release\.sh:' |
	grep -v 'with_legacy_session_mode' || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "banned wording found"
fi
say SUCCESS "no banned wording"

em_dash="$(printf '\342\200\224')"
hits=$(git ls-files -- crates tools ':(top,glob)*.md' | xargs grep -lI -- "$em_dash" 2>/dev/null || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "an em dash was found"
fi
say SUCCESS "no em dash"

hits=$(git ls-files -- ':(top,glob)*.md' | xargs grep -nE '^\s*\|.*\|\s*$' 2>/dev/null || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "a markdown table was found"
fi
say SUCCESS "no markdown table"

hits=$(git ls-files -- '*.rs' | xargs grep -nE '^\s*//' 2>/dev/null || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "a code comment was found in Rust source"
fi
say SUCCESS "no code comment"
