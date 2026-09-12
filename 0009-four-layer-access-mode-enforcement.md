# ADR-0009: Access modes are enforced in four layers, with PostgreSQL's own parser and a fail-closed policy

**Status**: Accepted (the three modes from the Principal Architect's brief; the enforcement design by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The brief requires read-only, write-only, and read-write modes. Every keyword or regex guard in the field was bypassed in 2026 (DBHub, AWS Labs, pgEdge, pgAdmin), and even a parser allowlist failed where it skipped a node kind (Postgres MCP Pro, CVE-2026-85620). AWS moved to a fail-closed allowlist on libpg_query on 2026-09-11; Microsoft removed its blocklists the same day and made the database role the authority. PostgreSQL's `READ ONLY` transaction blocks writes and DDL but still allows `SET`, `LISTEN`, `NOTIFY`, `VACUUM`, `REINDEX`, `CLUSTER`, `CALL`, `DO`, `LOCK`, and `ALTER SYSTEM`, and cannot stop a function with side effects. Write-only is not a PostgreSQL privilege model. `pg_query` 6.2.0 carries the PostgreSQL 17 grammar. How does OwnPG make a mode mean something?

## Decision Drivers

- No single layer is trusted; each one must fail independently before a mode is violated.
- Parse with PostgreSQL's grammar, never with a generic SQL parser or keywords.
- Fail closed on anything the parser cannot read, except where the user chose the most permissive mode.
- Local developers run as a superuser under `trust`; the tool must still be useful there and honest about it.
- Every decision leaves an audit record.

## Options Considered

### Option A: Four layers

1. Role boundary: at startup read the role's attributes and memberships; refuse superuser, `rds_superuser`, and `BYPASSRLS` when `strict_role = true` (the remote default); warn in `doctor` and flag audit records under the local default; ship least-privilege role templates for each mode (read-only: `USAGE` on the schema, `SELECT` on tables, `default_transaction_read_only = on`, `search_path = ''`, `pg_read_all_stats`).
2. Classification: parse every statement with `pg_query` (libpg_query); count statements with `split_with_parser` and refuse more than one per call; classify the parse tree into read, write, DDL, maintenance, or destructive; refuse any class the mode forbids; deny the escape-hatch functions in every mode (`pg_read_file`, `pg_ls_dir`, `lo_import`, `lo_export`, `dblink`, `COPY` to or from a file or `PROGRAM`, `pg_sleep`, `pg_terminate_backend` outside the maintenance tool, `session_replication_role`, `row_security`, `RESET ALL`, `DISCARD ALL`).
3. Engine controls per call: `BEGIN READ ONLY` with rollback in read-only mode, `statement_timeout`, `lock_timeout`, `idle_in_transaction_session_timeout` (derived from the transaction handle expiry) at session start, `search_path` pinned, one statement per call over the simple query protocol once the classifier has counted it.
4. Tool surface: `tools/list` returns only the mode's tools, in a fixed order; write tools never exist in read-only mode.

Parse-failure policy: a statement `pg_query` cannot parse is refused in read-only and write-only modes; in read-write mode it runs through the extended query protocol, which makes PostgreSQL itself refuse a multi-statement batch, and the audit record is flagged `unparsed`.

- Pros: a bypass must beat the role, the parser, the engine, and the tool list at once.
- Pros: matches where AWS and Microsoft landed on 2026-09-11, and adds the layers each of them lacks.
- Cons: `pg_query` lags PostgreSQL by one major (18 syntax may not parse yet); the classification map is derived and needs a bypass corpus in CI.

### Option B: Role boundary only (Microsoft's position)

- Pros: nothing to parse; PostgreSQL decides.
- Cons: local superusers get no protection at all; no write-only mode; no destructive detection; no audit of intent.

### Option C: Parser allowlist only (Postgres MCP Pro's position)

- Pros: fine-grained.
- Cons: one missed node kind is a CVE (CVE-2026-85620); function side effects are invisible to a parser.

### Option D: Keyword or regex guard

- Cons: bypassed everywhere in 2026; both AWS and Microsoft called it unsound.

### Option E: Do nothing (modes as documentation)

- Cons: the modes would be hints, and the survey shows agents run `DELETE` without `WHERE`.

## Decision

We will enforce every access mode in four independent layers (role boundary, libpg_query classification with a fail-closed policy, per-call engine controls, and a mode-specific tool list), refuse multi-statement input everywhere, and flag unparsed statements that run in read-write mode.

Option A won because each field failure of 2026 beat exactly one layer, and OwnPG's design requires beating four.

## Consequences

- Positive: read-only mode on a local superuser still has the parser, the transaction guard, and the tool list.
- Positive: write-only mode exists (top-level `SELECT` refused; `INSERT`, `UPDATE`, `DELETE`, `MERGE`, `COPY FROM STDIN` allowed with `RETURNING`).
- Negative: PostgreSQL 18-only syntax may be refused in read-only mode until `pg_query` ships the 18 grammar (PR #79); the README says so.
- Negative: a bypass corpus of at least 40 inputs runs against PostgreSQL 14 through 18 in CI and grows with every public advisory.
- Neutral: `EXPLAIN ANALYZE` of a write is refused in read-only mode and rolled back in read-write mode.

## Reversibility

Expensive to reverse: the classifier and the mode logic sit under every tool. Revisit the parse-failure policy when `pg_query` catches up with PostgreSQL 18, and revisit the deny list with every advisory.

## Sources

- https://github.com/awslabs/mcp/pull/4575 (merged 2026-09-11) and https://github.com/microsoft/mcp/pull/3202 (merged 2026-09-11)
- https://www.vulncheck.com/advisories/postgres-mcp-pro-0.3.0-restricted-mode-bypass-via-from-clause-function (2026-09-04)
- https://aws.amazon.com/security/security-bulletins/2026-101-aws/ (2026-09-04)
- https://www.postgresql.org/docs/18/sql-set-transaction.html and `src/backend/tcop/utility.c` on `REL_18_STABLE`
- https://www.postgresql.org/docs/18/predefined-roles.html
- https://github.com/pganalyze/pg_query.rs/blob/main/CHANGELOG.md (6.2.0, 2026-07-29) and pull request #79
- `research/03-existing-postgresql-mcp-servers.md`, question 3; `research/05-postgresql-18-dba-surface.md`, question 7
