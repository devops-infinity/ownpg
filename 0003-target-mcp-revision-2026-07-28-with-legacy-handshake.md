# ADR-0003: Target MCP revision 2026-07-28 and keep the 2025-11-25 handshake for older clients

**Status**: Accepted (the Principal Architect asked for the latest specification as of today)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The current MCP revision is `2026-07-28` (stable release on 2026-07-28T16:47:49Z). It removed sessions, the `initialize` handshake, `ping`, the HTTP GET stream, and resumability; it added `server/discover`, per-request `_meta`, `resultType`, caching hints, required HTTP headers, and the multi-round-trip pattern for elicitation. The last official client matrix (2026-05-26) predates the release, and rmcp's default handshake target is still `2025-11-25` because that is the newest revision with a handshake. Which revisions does OwnPG speak?

## Decision Drivers

- The Principal Architect's instruction: the latest specification as of today.
- Clients in daily use (Claude Code, Claude Desktop, Cursor, VS Code, Codex) may still open with `initialize`.
- The spec's own dual-era guidance: a request with modern `_meta` is served statelessly; an `initialize` request selects legacy semantics.
- The conformance suite scores `2026-07-28` and `2025-11-25` separately.

## Options Considered

### Option A: 2026-07-28 first, 2025-11-25 handshake kept

Serve `server/discover` and per-request `_meta` as the primary path; answer `initialize` with `2025-11-25` semantics through rmcp's `negotiate_initialize` and `legacy_session_mode`; never adopt the HTTP+SSE transport.

- Pros: current spec, current clients, and older clients all work.
- Pros: rmcp already implements both eras.
- Cons: two code paths to test; the legacy path carries sessions on HTTP.

### Option B: 2026-07-28 only, reject legacy

Supabase's self-hosted server does this (`legacy: 'reject'`).

- Pros: one code path, no sessions anywhere.
- Cons: any client that still opens with `initialize` gets an error until it upgrades; risky in September 2026.

### Option C: 2025-11-25 only

- Pros: the widest installed base today.
- Cons: contradicts the instruction; misses caching hints, header validation, and MRTR; deprecated primitives would be built in.

### Option D: Do nothing (no version policy)

- Cons: rmcp's defaults would decide, silently, and the tests would not know what to assert.

## Decision

We will target `2026-07-28` as the primary protocol and keep the `2025-11-25` handshake path alive for older clients, with the HTTP+SSE transport excluded.

Option A won because it satisfies the instruction and the installed base at once, and because the spec itself describes exactly this dual-era server.

## Consequences

- Positive: `tools/list` carries `ttlMs` and `cacheScope`, tool results carry `resultType`, and header validation follows the current rules.
- Positive: `reserve`'s dual-era tests become OwnPG's acceptance tests.
- Negative: two conformance runs in CI (`--spec-version 2026-07-28` and `2025-11-25`).
- Neutral: Roots, Sampling, and protocol Logging are deprecated and are not used; the legacy path may be removed twelve months after a future revision marks it removed.

## Reversibility

Cheap to reverse: dropping the legacy path is a configuration and test change once every target client speaks `2026-07-28`. Revisit at each spec revision and when the client matrix shows no legacy clients in use.

## Sources

- https://modelcontextprotocol.io/specification/2026-07-28/changelog (2026-07-28)
- https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning
- https://github.com/modelcontextprotocol/rust-sdk/pull/1228 (merged 2026-08-31)
- `ownpg-research-report.md`, section 3; `research/01-mcp-specification-2026-07-28.md`
- `/Users/sharkar/Git-Repositories/reserve/CHANGELOG.md:15-17`
