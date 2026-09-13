#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

say() { printf '[%s] %s\n' "$1" "$2"; }

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push tools/verify.sh
say SUCCESS "git now runs .githooks/pre-commit and .githooks/pre-push from this repository"
