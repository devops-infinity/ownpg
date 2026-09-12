# ADR-0010: One database and one schema per run, with a pinned search_path

**Status**: Accepted (scoping from the Principal Architect's brief)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The brief says each run serves one project and one database schema. No surveyed server enforces a single schema at the server level; DBHub sets `search_path`, Supabase and Neon scope by project. The CVE-2018-1058 guidance says a client should set `search_path` as the first command of a session, and PostgreSQL 15 removed `CREATE` on `public` from `PUBLIC` for new databases. Poolers in transaction mode do not keep a session-level `search_path`. How is the scope defined and enforced?

## Decision Drivers

- A second project is a second process, never a second argument.
- Generated SQL must be safe against search_path hijacking.
- The scope must hold behind PgBouncer and Supavisor.
- Catalog reads (`pg_catalog`, `information_schema`) must stay possible.

## Options Considered

### Option A: Startup-bound database and schema, pinned search_path, qualified names, classifier check

`--database` and `--schema` (default `public`) are required at startup and never change; the session's first statement is `SELECT pg_catalog.set_config('search_path', '<schema>', false)` (or `''` with fully qualified names when `pooled = true`, applied as `SET LOCAL` inside each transaction); every generated statement qualifies object names with the scoped schema through a `quote_ident` equivalent; the classifier refuses any `RangeVar` whose schema is another user schema and refuses unqualified names when the pooled mode runs with an empty `search_path`; reads of `pg_catalog` and `information_schema` are allowed; `doctor` warns when `public` still grants `CREATE` to `PUBLIC`.

- Pros: the scope is a property of the process, visible in every client config.
- Pros: hijack-safe and pooler-safe.
- Cons: cross-schema joins are refused, which some users will want.

### Option B: Schema as a tool argument

- Pros: one process for many schemas.
- Cons: the model chooses the scope; the mode and the audit lose their anchor; the brief says otherwise.

### Option C: Database-wide scope with search_path only

- Pros: simplest.
- Cons: nothing stops a query on another schema; search_path alone is the CVE-2018-1058 trap.

### Option D: Do nothing

- Cons: PostgreSQL's default `$user, public` search_path decides.

## Decision

We will bind each process to one database and one schema at startup, pin `search_path` as the first statement of every session (per transaction when pooled), qualify every generated name, and refuse references to other user schemas in the classifier.

Option A won because the brief asks for it and because it closes the search_path class of attacks by construction.

## Consequences

- Positive: client configs read `ownpg serve --database app --schema public`, which documents the scope.
- Positive: audit records carry a fixed database and schema.
- Negative: users who need two schemas run two servers with distinct names.
- Neutral: system catalog reads remain available to the exploration and monitoring tools.

## Reversibility

Cheap to reverse: a later ADR could allow an allowlist of extra schemas per profile without changing the default.

## Sources

- https://wiki.postgresql.org/wiki/A_Guide_to_CVE-2018-1058%3A_Protect_Your_Search_Path
- https://www.postgresql.org/docs/15/release-15.html and https://www.postgresql.org/docs/18/ddl-schemas.html
- https://www.pgbouncer.org/features.html and https://github.com/supabase/supavisor/issues/206
- `research/05-postgresql-18-dba-surface.md`, finding 7.3; `research/03-existing-postgresql-mcp-servers.md`, question 7
