#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

source "$REPO/tools/lib.sh"

SHELL_FILES=(tools/*.sh .githooks/pre-commit .githooks/pre-push)
LOG="$(mktemp)"
STEP="startup"

on_interrupt() {
	say FAILED "interrupted during: $STEP"
	exit "$1"
}
trap 'on_interrupt 130' INT
trap 'on_interrupt 143' TERM
trap 'rm -f "$LOG"' EXIT

run_check() {
	STEP=$1
	local hint=$2
	shift 2
	if ! "$@" >"$LOG" 2>&1; then
		say FAILED "$STEP"
		tail -n 40 "$LOG" >&2
		die "$hint"
	fi
}

bash -n "${SHELL_FILES[@]}" || die "a shell script has a syntax error"
say SUCCESS "syntax"

command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH"

say INFO "running the verification gate"
run_check "formatting" "formatting is not clean; run: cargo fmt --all" \
	cargo fmt --all -- --check
say SUCCESS "formatting"
run_check "check" "the check failed; run: cargo check --workspace --all-targets --all-features" \
	cargo check --workspace --all-targets --all-features --locked --color=never
say SUCCESS "check"
run_check "lint" "clippy found problems; run: cargo clippy --workspace --all-targets --all-features -- -D warnings" \
	cargo clippy --workspace --all-targets --all-features --locked --color=never -- -D warnings
say SUCCESS "lint"
if command -v cargo-nextest >/dev/null 2>&1; then
	run_check "tests" "tests failed; run: cargo nextest run --workspace --all-features" \
		cargo nextest run --workspace --all-features --locked --no-tests=warn --color=never
else
	run_check "tests" "tests failed; run: cargo test --workspace --all-features" \
		cargo test --workspace --all-features --locked
fi
say SUCCESS "tests"
run_check "doc tests" "doc tests failed; run: cargo test --workspace --all-features --doc" \
	cargo test --workspace --all-features --locked --doc
say SUCCESS "doc tests"
run_check "docs" "the docs do not build; run: RUSTDOCFLAGS=\"-D warnings\" cargo doc --workspace --all-features --no-deps" \
	env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
say SUCCESS "docs"
for tool in cargo-audit cargo-deny cargo-machete; do
	command -v "$tool" >/dev/null 2>&1 ||
		die "$tool is not installed; install it with: cargo install --locked $tool"
done
run_check "advisories" "cargo audit found an advisory; run: cargo audit --deny warnings" \
	cargo audit --deny warnings
say SUCCESS "advisories"
STEP="dependency policy"
DENY_CODE=0
cargo deny check >"$LOG" 2>&1 || DENY_CODE=$?
if [[ $DENY_CODE -ne 0 ]]; then
	tail -n 40 "$LOG" >&2
	DENY_FAILED_CHECKS=""
	((DENY_CODE & 1)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS advisories"
	((DENY_CODE & 2)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS bans"
	((DENY_CODE & 4)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS licenses"
	((DENY_CODE & 8)) && DENY_FAILED_CHECKS="$DENY_FAILED_CHECKS sources"
	die "cargo deny found a policy violation in:${DENY_FAILED_CHECKS:- an unrecognized check (exit $DENY_CODE)}; run: cargo deny check"
fi
say SUCCESS "dependency policy"
run_check "no unused dependency" "cargo machete found an unused dependency; run: cargo machete" \
	cargo machete
say SUCCESS "no unused dependency"

if command -v shellcheck >/dev/null 2>&1 && command -v shfmt >/dev/null 2>&1; then
	run_check "shell scripts" "shellcheck found a problem; run: shellcheck -x -P . ${SHELL_FILES[*]}" \
		shellcheck -x -P . "${SHELL_FILES[@]}"
	run_check "shell scripts" "shfmt found unformatted shell; run: shfmt -w ${SHELL_FILES[*]}" \
		shfmt -d "${SHELL_FILES[@]}"
	say SUCCESS "shell scripts"
else
	say WARNING "shellcheck or shfmt is not installed, so the shell scripts were not checked"
fi

if command -v bats >/dev/null 2>&1; then
	run_check "shell script tests" "the bats suite failed; run: bats tools/tests/toolset.bats" \
		bats tools/tests/toolset.bats
	say SUCCESS "shell script tests"
else
	say WARNING "bats is not installed, so tools/tests/toolset.bats was not run"
fi

hits=$(git ls-files -z -- crates tools ':(top,glob)*.md' ':(exclude)**/LICENSE-*' |
	xargs -0 grep -niIE 'legacy|backward.compat|inspired by|based on|ported from|fork of' 2>/dev/null |
	grep -v '^tools/release\.sh:' |
	grep -v '^tools/verify\.sh:' |
	grep -v -i 'with_legacy_session_mode' || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "banned wording found"
fi
say SUCCESS "no banned wording"

if [[ -f "$HOME/.claude/rules/banned-words.md" ]]; then
	banned_terms=$(awk '/MACHINE-CHECKED LIST BEGIN/,/MACHINE-CHECKED LIST END/' "$HOME/.claude/rules/banned-words.md" |
		sed -n 's/^- \([^(]*\) (.*)$/\1/p' | sed 's/[[:space:]]*$//')
	hits=$(git ls-files -z -- ':(top,glob)*.md' |
		xargs -0 grep -niIF -f <(printf '%s\n' "$banned_terms") 2>/dev/null || true)
	if [[ -n "$hits" ]]; then
		printf '%s\n' "$hits"
		die "AI-footprint wording found in a markdown file; see \$HOME/.claude/rules/banned-words.md"
	fi
	say SUCCESS "no AI-footprint wording"
else
	say WARNING "\$HOME/.claude/rules/banned-words.md is not present, so AI-footprint wording was not checked"
fi

em_dash="$(printf '\342\200\224')"
hits=$(git ls-files -z -- crates tools ':(top,glob)*.md' | xargs -0 grep -lI -- "$em_dash" 2>/dev/null || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "an em dash was found"
fi
say SUCCESS "no em dash"

hits=$(git ls-files -z -- ':(top,glob)*.md' | xargs -0 grep -nE '^\s*\|.*\|\s*$' 2>/dev/null || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "a markdown table was found"
fi
say SUCCESS "no markdown table"

hits=$(git ls-files -z -- '*.rs' | xargs -0 grep -nE '^\s*//|[[:space:]]//' 2>/dev/null || true)
if [[ -n "$hits" ]]; then
	printf '%s\n' "$hits"
	die "a code comment was found in Rust source"
fi
say SUCCESS "no code comment"
