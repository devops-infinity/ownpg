# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/).

## [Unreleased]

### Added

- An MCP (Model Context Protocol) server that serves one PostgreSQL database and one schema, over stdio by default or over Streamable HTTP with `--http`.
- Three access modes: read-only, write-only, and read-write, set with `--mode`.
- Three HTTP authentication modes: none (loopback only), bearer token, and OAuth, set with `--auth`.
- Write and transaction tools that load automatically once the access mode allows writes, plus five optional tool groups beyond the default objects, read, and health tools: DDL, roles, maintenance, monitoring, and host programs, loaded with `--tools`.
- A strict-role check that refuses to start on a superuser, an rds_superuser member, or a role with `BYPASSRLS`, on by default in HTTP mode, controlled with `--strict-role`.
- Connectivity to PostgreSQL over TCP, a Unix socket, or an SSH bastion host, with an in-process SSH client or the system `ssh` command selectable with `--ssh-transport`.
- An append-only, hash-chained audit log, on by default, verified with `ownpg audit verify`, controlled with `--no-audit` and `--audit-path`.
- `ownpg doctor`, a connection and settings check with text or JSON output.
- `ownpg config`, connection-profile management with OS-keychain-backed password and SSH-passphrase storage.
- `ownpg man`, a generated manual page for the whole tool or one subcommand.
- `ownpg completions`, shell completion scripts for bash, elvish, fish, powershell, and zsh.
- Daily log-file rotation, keeping the newest eight files, with `--log-format text|json`.
- `audit_on_failure` (`refuse-writes` by default, or `refuse-all` or `continue`), which decides what runs while the audit log cannot be written; a gap marker in the chain counts the lines that were lost.
- `audit_keep_days` (366 by default), which removes closed audit files of the same profile after that many days and records each removal in the chain.
- `keychain_scope` (`file` by default, or `target`), which binds a stored password or SSH secret to the profile file and the server it was saved for.
- A dependency check on `DROP ... CASCADE`: a dry run and the confirmation prompt list every object the cascade would also drop, and a cascade that reaches outside the served schema is refused.
- A per-caller HTTP rate limit of 300 requests a minute by default, set with `OWNPG_RATE_LIMIT_PER_MINUTE` (`0` turns it off).
- Cursor paging for `resources/list`, so a schema with more than 1,000 tables is listed in full.
- `transaction_size` on `pg_restore`, and new `inserted_since_vacuum`, `insert_threshold`, and `transaction_id_age` fields on `pg_vacuum_needs`.
- Metrics for dropped audit lines (`ownpg.audit.dropped`), a failing audit log (`ownpg.audit.degraded`), and refused HTTP requests (`ownpg.http.rejections`).
- Connection pool metrics named after the OpenTelemetry conventions: `db.client.connection.count` by state, `db.client.connection.max`, and `db.client.connection.pending_requests`.
- `ownpg health`, which asks a running HTTP server whether it is ready and exits 0 or 1.
- `result_text` (`full` by default, or `summary`), which replaces the text copy of a long tool result with one line, for servers whose clients all read `structuredContent`.
- Publications and event triggers in `pg_list_objects`, each publication's actions and published tables in `pg_replication`, and a `list_defaults` operation on `pg_privileges` for default privileges.
- An audit line for every transaction handle the server closes on its own: after its idle expiry, at `transaction_timeout_seconds`, when its connection drops, and at shutdown.

### Changed

- `json` and `jsonb` cells are returned as JSON values instead of escaped strings, unless a number in them would lose precision or a key repeats.
- `pg_restore` runs as one transaction by default and adds `--if-exists` whenever `clean` is set; `pg_dumpall_globals` leaves role password hashes out by default.
- `pg_role` hashes a password with the server's `password_encryption` method before sending it.
- `SELECT ... FOR UPDATE` and transaction-scoped advisory locks count as writes, and write-only mode refuses any `SELECT` that returns rows.
- `CREATE PUBLICATION ... FOR ALL TABLES` needs confirmation.
- A confirmation is valid only for the database it was asked for and only on the server run that issued it.
- The OAuth issuer is compared exactly as configured, a key endpoint outage answers `503` with `Retry-After`, and a request without credentials gets a challenge without an error code.
- A transaction handle ends at `transaction_timeout_seconds` even while it keeps running statements, on every supported PostgreSQL version.
- A `COMMIT` cut off by a dropped connection is settled from the server's record of the transaction, so it comes back committed or rolled back; `commit.outcome_unknown` is left for a check that gets no answer, and it names the transaction ID.
- Statement text in `pg_activity`, `pg_locks`, and `pg_top_queries` shows `?` in place of string and number literals.
- The container health check runs `ownpg health` against the running server instead of `ownpg doctor`.

### Removed

- `ownpg config cache-clear` and the cache directory, which held nothing.
