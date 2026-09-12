# ADR-0007: tokio-postgres with rustls as the PostgreSQL driver

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

OwnPG runs SQL it cannot know ahead of time, returns rows whose types it cannot predict (including catalog types such as `aclitem`), needs `COPY` in both directions, needs cancellation from another task, and must accept a connection over an SSH channel. `owndbs` uses `sqlx` for dump and restore; `reserve` has no database. The two candidates are `tokio-postgres` 0.7.18 (2026-06-12, MSRV 1.85) and `sqlx` 0.9.0 (2026-05-21, MSRV 1.94). Which driver, and which TLS backend?

## Decision Drivers

- Text-mode results for arbitrary SQL, with statement framing preserved.
- A raw-stream constructor for the SSH tunnel.
- Behavior behind poolers (no named prepared statements).
- MSRV alignment with rmcp (1.88) and russh (1.89).
- A pure-Rust TLS stack, as `reserve` and `owndbs` already use.

## Options Considered

### Option A: tokio-postgres 0.7.x with tokio-postgres-rustls 0.14

`simple_query` returns every column as text, runs semicolon-separated statements in one round trip, and keeps the framing; `query_typed` runs parameterized statements in one round trip without a named prepared statement; `connect_raw` takes any `AsyncRead + AsyncWrite` stream; `copy_in` and `copy_out`; `cancel_token`; `check_connection`; keepalives on by default. `sslmode` accepts only `disable`, `prefer`, `require`, so OwnPG maps `allow`, `verify-ca`, and `verify-full` onto the rustls `ClientConfig` itself (the rustls default already behaves like `verify-full`). Channel binding works since tokio-postgres-rustls 0.14.0.

- Pros: text mode avoids the binary-output failure on catalog types; framing preserved for scripts; raw stream for SSH.
- Pros: MSRV 1.85; small dependency surface; pooler-safe paths built in.
- Cons: no libpq environment or file handling (ADR-0006 covers it); `sslmode` mapping is OwnPG's code.

### Option B: sqlx 0.9.0

- Pros: reads `PG*` variables and the password file; `verify-ca` and `verify-full` built in; compile-time checked queries for OwnPG's own catalog SQL.
- Cons: `query()` prepares everything and asks for binary output on every column, which fails on `aclitem` (issue #1269, open since 2021); `raw_sql` wraps a user's script in an implicit transaction and changes its meaning; custom types in records unsupported; MSRV 1.94; the repository moved to a new organization in 2026.

### Option C: Both (sqlx for OwnPG's own catalog queries, tokio-postgres for user SQL)

- Pros: compile-time checks on the catalog queries.
- Cons: two connections or two drivers over one socket, two TLS stacks to configure, MSRV 1.94 anyway.

### Option D: Do nothing

- Cons: no database access.

## Decision

We will use `tokio-postgres` with `tokio-postgres-rustls` (crypto backend `ring`, `native-certs` and `webpki-roots` features on), `simple_query` for every user statement the classifier has parsed, the extended protocol (`query_raw`) only for a statement the classifier could not parse in read-write mode so PostgreSQL itself refuses a batch, `query_typed` for OwnPG's own parameterized catalog queries, and `connect_raw` for SSH.

Option A won because text-mode, framing-preserving execution is what a DBA tool needs, and because the SSH design (ADR-0008) needs `connect_raw`.

## Consequences

- Positive: every result column is a string the tool can shape without a type table.
- Positive: `CopyInSink` and `CopyOutStream` cover `COPY` over the connection.
- Negative: OwnPG maps the six `sslmode` values and the `sslrootcert=system` rule itself: `prefer` is the default as in libpq, `require` without a root certificate uses a deliberate no-verify verifier as libpq does, `verify-ca` and `verify-full` fall back to the platform store and then `webpki-roots`, and one stderr warning per process reports an unverified or unencrypted connection (decided 2026-09-12).
- Neutral: `client_encoding` is set to UTF8 at connection start so every text column decodes without a per-database check.
- Negative: OwnPG's own catalog queries have no compile-time check; they get integration tests on PostgreSQL 14 through 18 instead.
- Neutral: `deadpool-postgres` 0.14.2 is added only for the remote multi-client mode.

## Reversibility

Expensive to reverse: result shaping, catalog queries, and the SSH path are written against the driver's API. Revisit if `sqlx` gains a text-protocol result path that keeps script framing.

## Sources

- https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html (0.7.18)
- https://github.com/rust-postgres/rust-postgres/issues/596 and https://github.com/transact-rs/sqlx/issues/1269
- https://docs.rs/sqlx/latest/sqlx/fn.raw_sql.html (0.9.0)
- https://crates.io/crates/tokio-postgres-rustls (0.14.0, 2026-05-21)
- `research/04-rust-postgresql-connectivity-and-ssh.md`, findings 1 to 9 and 30 to 32
- `/Users/sharkar/Git-Repositories/owndbs/Cargo.toml:26-31` (sqlx use in the dump tool, read 2026-09-12)
