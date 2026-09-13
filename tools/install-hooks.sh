#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

source "$REPO/tools/lib.sh"

command -v git >/dev/null 2>&1 || die "git is not on PATH"

git config core.hooksPath .githooks || die "could not set core.hooksPath; is this a git repository?"
chmod +x .githooks/pre-commit .githooks/pre-push tools/verify.sh ||
	die "could not make the hook scripts executable"
say SUCCESS "git now runs .githooks/pre-commit and .githooks/pre-push from this repository"
