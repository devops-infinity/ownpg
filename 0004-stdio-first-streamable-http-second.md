# ADR-0004: stdio is the primary transport and Streamable HTTP is the secondary one

**Status**: Accepted (ordering from the Principal Architect's brief)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The brief names a local server against a local PostgreSQL as the primary mode and a remote server as the secondary one. The `2026-07-28` specification defines two transports, stdio and Streamable HTTP, and marks HTTP+SSE deprecated. The spec's security page says a local server should "Use the `stdio` transport to limit access to just the MCP client" and that an HTTP server should require a token and bind to localhost. Which transport is default, and how is the other exposed?

## Decision Drivers

- Local first: every daily client (Claude Code, Claude Desktop, Cursor, VS Code, Codex, Gemini CLI, Zed) launches stdio servers.
- Authorization does not apply to stdio; credentials come from the environment.
- HTTP without auth was the cause of CVE-2026-61742 (DBHub) and CVE-2026-59971 (mysql-mcp-server bound to 0.0.0.0).
- One binary, one command surface.

## Options Considered

### Option A: `ownpg serve` is stdio; `ownpg serve --http` is Streamable HTTP

The HTTP listener binds 127.0.0.1 by default, validates Origin and Host, refuses a non-loopback bind without an auth mode, and carries the remote-mode features from ADR-0014.

- Pros: the safe transport is the default; remote mode is one flag away.
- Pros: the same handlers serve both.
- Cons: two transports to test in CI (the conformance suite needs HTTP anyway).

### Option B: HTTP only, with `mcp-remote` for stdio clients

- Pros: one transport.
- Cons: every local user runs a Node bridge; the spec's local-server guidance is ignored; an open port on every laptop.

### Option C: stdio only

- Pros: smallest surface.
- Cons: no remote mode, no conformance suite run (it needs a URL), no claude.ai connector.

### Option D: Do nothing

- Cons: the choice would fall to rmcp's examples by accident.

## Decision

We will make stdio the default transport behind `ownpg serve`, and expose Streamable HTTP behind `serve --http` with loopback binding and mandatory authentication for any other address.

Option A won because it follows the spec's local-server guidance and the brief's ordering while keeping one binary.

## Consequences

- Positive: local use never opens a port.
- Positive: the conformance suite runs against the HTTP transport in CI, so both transports stay proven.
- Negative: the HTTP path carries the header validation, Origin checks, body cap, and bearer or OAuth middleware.
- Neutral: HTTP+SSE is never implemented.

## Reversibility

Cheap to reverse: the default is a flag. Revisit if the roadmap's "Streamable HTTP over stdio" idea lands in a future revision.

## Sources

- https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio
- https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http
- https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices
- `research/03-existing-postgresql-mcp-servers.md`, question 5 (CVE-2026-61742, CVE-2026-59971)
