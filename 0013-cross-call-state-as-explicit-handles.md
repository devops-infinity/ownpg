# ADR-0013: Cross-call state lives in explicit, server-minted handles

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The `2026-07-28` protocol is stateless: no sessions, no connection context, and the spec says "Servers MUST NOT rely on prior requests over the same connection to establish context." Its own stateful-tools guidance names "a database transaction" as the case for explicit handles: return one from a creation tool, accept it on later calls, check authorization every time, keep it opaque, state its lifetime, and report expiry as a tool error. A DBA tool needs transactions that span several calls, cursors for paginated reads, and long maintenance jobs. The Tasks extension exists but rmcp's push path for it is not wired. How does OwnPG keep state?

## Decision Drivers

- Spec compliance on statelessness.
- A handle is a name, never a credential: possession must not grant access.
- Expiry must be visible and bounded.
- The local single-connection mode and the remote pooled mode must share the design.

## Options Considered

### Option A: Opaque handles with a registry keyed by principal

`pg_transaction` with `operation: begin` returns an opaque random handle; `commit`, `rollback`, `savepoint`, and `rollback_to` take it; the registry key is `principal:handle`; each handle owns a pinned connection (the single connection in local mode, a checked-out pooled connection in remote mode), a configurable idle expiry (default 60 seconds, aligned with `idle_in_transaction_session_timeout`), and a description of its lifetime in the tool text. Read cursors from `pg_run_query` are server-side `DECLARE CURSOR` handles inside the read transaction, with a shorter expiry and a cap of eight in local mode. Every handle carries a state (`open`, `expired`, `committed`, `rolled_back`, `lost`) with a written transition table; expiry rolls the transaction back, a connection drop marks the handles on it `lost`, a call on a non-open handle gets an `isError` result naming the state, and `idle_in_transaction_session_timeout` is set to the expiry plus five seconds so the handle always expires before PostgreSQL aborts the transaction. Long maintenance jobs run to completion within the call in 1.0, honoring cancellation; the Tasks extension is deferred (PRD section 14).

- Pros: exactly the spec's guidance; works on stdio and HTTP alike.
- Pros: audit records tie every statement to a handle.
- Cons: the local mode's connection is busy while a transaction handle is open, so reads and catalog tools use a second connection that opens lazily (two connections at most).

### Option B: Implicit session state (a transaction stays open on the connection)

- Pros: fewer arguments.
- Cons: violates the stateless rule; invisible to the audit; leaks across clients in remote mode.

### Option C: Tasks extension for everything long-running

- Pros: polling and cancellation defined by the spec.
- Cons: only VS Code and Copilot CLI listed Tasks support; rmcp's push path is not wired; premature for 1.0.

### Option D: Do nothing (no transactions spanning calls)

- Cons: multi-statement schema changes could not be grouped; users would lose atomicity.

## Decision

We will hold transactions and read cursors in opaque, principal-bound handles with visible expiry, keep long operations in-call with cancellation in 1.0, and defer the Tasks extension.

Option A won because the spec prescribes it and both transports support it without protocol state.

## Consequences

- Positive: `begin`, three DDL calls, `commit` is a real atomic change with an audit trail.
- Positive: pagination through cursor handles keeps result sizes bounded.
- Negative: local mode holds up to two connections while a transaction handle is open; the second opens on the first read that needs it and closes with the handle (decided 2026-09-12, no longer open for M2).
- Neutral: the Tasks extension is re-evaluated at M3.

## Reversibility

Cheap to reverse for cursors; expensive for transactions once users script them. Revisit when rmcp wires task notifications and the client matrix shows Tasks support in the daily clients.

## Sources

- https://modelcontextprotocol.io/specification/2026-07-28/basic#statelessness
- https://modelcontextprotocol.io/specification/2026-07-28/server/tools (stateful tools guidance)
- https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices (state handle hijacking)
- https://github.com/modelcontextprotocol/rust-sdk/pull/1020 (tasks limitation)
- `research/01-mcp-specification-2026-07-28.md`, findings 5.2, 6.5, 7.13
