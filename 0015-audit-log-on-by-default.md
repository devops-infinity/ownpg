# ADR-0015: A JSON-lines audit log is on by default, and protocol logging is not used

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

DBHub, Postgres MCP Pro, the Google Toolbox, and the reference server keep no audit log of executed statements; the OWASP MCP Top 10 (MCP08) and the NSA's MCP guidance ask for a traceable record of every call; the Principal Architect's constitution requires an audit trail for every run. The `2026-07-28` revision deprecates protocol logging and says to log to stderr on stdio and through OpenTelemetry on HTTP. What does OwnPG record, where, and what does it never record?

## Decision Drivers

- Every tool call and every statement decision is traceable after the fact.
- Values, rows, passwords, and tokens never reach the log.
- The log survives a crash and is easy to grep.
- The house rule: never write a secret or a connection string into a log.

## Options Considered

### Option A: Append-only JSON-lines file in the OwnPG data directory, on by default

One line per tool call: `v` (1), `request_id` (the `_meta` trace id when present, otherwise generated, and shared with the stderr span), timestamp (RFC 3339 with offset), tool, operation, mode, transport, principal and `principal_kind` (`local`, `os_user`, `token_subject`, `bearer`), database, schema, statement class, normalized statement (with literals replaced) or its `pg_query` fingerprint when longer than a cap, decision (`allowed`, `refused`, `dry_run`, `confirmed_argument`, `confirmed_elicitation`, `unparsed`), the rule that decided, handle id, integer `duration_ms`, row count, truncated flag, outcome (SQLSTATE on error), the superuser flag, and `prev`, the SHA-256 of the previous line, so a removed or edited line breaks the chain. One file per process, named by profile and process id, opened once in append mode with mode 0600 under a 0700 data directory (ADR-0016), rotated by size inside OwnPG with a marker line that starts a new chain, overridable, and `audit = false` to turn it off. Runtime logs go to stderr through `tracing` with `RUST_LOG`; HTTP mode can export OpenTelemetry and forwards `_meta` trace context. Protocol `notifications/message` is never emitted. pgAudit on the database side is documented as the complementary server-side record.

- Pros: greppable, durable, secret-free by construction.
- Pros: matches the constitution's audit requirement and MCP08.
- Cons: the normalizer must be correct so literals never leak; disk use on busy servers.

### Option B: stderr only

- Pros: nothing to configure.
- Cons: stderr is lost when the client discards it; no durable record.

### Option C: Audit into a PostgreSQL table

- Pros: queryable with SQL.
- Cons: a write into the scoped database from a read-only tool; the audit would live inside the thing being audited.

### Option D: Do nothing

- Cons: MCP08 and the constitution both forbid it.

## Decision

We will write an append-only JSON-lines audit log by default, with literals normalized away and no values, rows, or secrets, keep runtime logs on stderr and OpenTelemetry, and never use protocol logging.

Option A won because it is durable, secret-free, and the only option outside the audited database.

## Consequences

- Positive: "what did the assistant run today" is a one-line grep.
- Positive: every refusal is recorded with its rule, which is how the bypass corpus grows.
- Negative: the normalizer (built on `pg_query::normalize` and `fingerprint`) needs tests that prove literal removal.
- Neutral: OwnPG rotates by size itself; the README also gives a `logrotate` example, states that the log holds principal identifiers, and documents the erasure procedure.

## Reversibility

Cheap to reverse: the sink is one trait implementation.

## Sources

- https://owasp.org/www-project-mcp-top-10/ (MCP08, v0.1)
- https://media.defense.gov/2026/Jun/02/2003943289/-1/-1/0/CSI_MCP_SECURITY.PDF (2026-05-20)
- https://modelcontextprotocol.io/specification/2026-07-28/changelog (Logging deprecated, SEP-2577)
- https://github.com/pgaudit/pgaudit
- `~/.claude/skills/constitution/constitution.md`, Article XIII
- `research/06-mcp-tool-design-security-clients-distribution.md`, finding 22
