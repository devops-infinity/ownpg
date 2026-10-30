#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BASE="${1:-http://127.0.0.1:8765}"
source "$REPO/tools/lib.sh"
require_bash 4.4 "$REPO/tools/smoke.sh" "$@"
shopt -s inherit_errexit

command -v curl >/dev/null 2>&1 || die "curl is not installed; install it with: brew install curl"
command -v python3 >/dev/null 2>&1 || die "python3 is not installed; install it with: brew install python3"

TOKEN_HEADER=()
if [[ -n "${OWNPG_BEARER_TOKENS:-}" ]]; then
	TOKEN_HEADER=(-H "Authorization: Bearer ${OWNPG_BEARER_TOKENS%%,*}")
fi
PROTOCOL_VERSION="${OWNPG_PROTOCOL_VERSION:-2025-11-25}"

check() {
	local name="$1"
	shift
	if "$@" >/dev/null 2>&1; then
		say SUCCESS "$name"
	else
		die "$name failed against $BASE"
	fi
}

check "healthz" curl -fsS "$BASE/healthz/live"
check "readyz" curl -fsS "$BASE/healthz/ready"

initialize_body="$(mktemp)"
tools_body="$(mktemp)"
trap 'rm -f "$initialize_body" "$tools_body"' EXIT

curl -fsS -X POST "$BASE/mcp" \
	"${TOKEN_HEADER[@]}" \
	-H 'content-type: application/json' \
	-H 'accept: application/json, text/event-stream' \
	-d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"$PROTOCOL_VERSION\",\"capabilities\":{},\"clientInfo\":{\"name\":\"ownpg-smoke\",\"version\":\"0\"}}}" \
	>"$initialize_body"

curl -fsS -X POST "$BASE/mcp" \
	"${TOKEN_HEADER[@]}" \
	-H 'content-type: application/json' \
	-H 'accept: application/json, text/event-stream' \
	-H "MCP-Protocol-Version: $PROTOCOL_VERSION" \
	-d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
	>"$tools_body"

python3 - "$initialize_body" "$tools_body" <<'PYJSON'
import json
import sys

with open(sys.argv[1]) as handle:
    handshake = json.load(handle)
server = handshake.get("result", {}).get("serverInfo", {})
if not server.get("version"):
    raise SystemExit("initialize answered without a server version")

with open(sys.argv[2]) as handle:
    listing = json.load(handle)
tools = listing.get("result", {}).get("tools", [])
if not tools:
    raise SystemExit("tools/list answered with no tools")
print(f"server {server['version']}, {len(tools)} tools")
PYJSON
say SUCCESS "mcp handshake and tools/list"
say SUCCESS "smoke test passed against $BASE"
