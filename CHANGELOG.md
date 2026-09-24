# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.1] - 2026-09-24

### Added

- `minisign.pub` at the repository root, the public key that checks the minisign signature on each release's `sha256.sum`.

### Fixed

- `ownpg --version` on a crates.io install shows the build date, not `built unknown`.
- A build from a git worktree shows its commit and build date, not `unknown`.
- OwnPG builds for Windows. There, a Unix socket directory as `--host` and `--ssh-transport system` each report that they need macOS or Linux.

## [0.1.0] - 2026-09-24

### Added

- An MCP (Model Context Protocol) server that serves one PostgreSQL database and one schema, over stdio by default or over Streamable HTTP with `--http`.
- Three access modes, `read-only` (the default), `write-only`, and `read-write`, set with `--mode`.
- Statement checking with PostgreSQL's own parser: a statement the access mode does not allow, a statement that names an object outside the served schema, and a call carrying more than one statement are refused.
- `SELECT ... FOR UPDATE` and transaction-scoped advisory locks count as writes, and write-only mode refuses a `SELECT` that returns rows.
- A confirmation gate for destructive statements, answered with `confirm: true` or an MCP elicitation prompt, and bound to the statement, the caller, and the host, port, database, and schema for 300 seconds.
- `dry_run` on the write tools, which shows the exact statement and every object a `DROP ... CASCADE` would also drop.
- A `DROP ... CASCADE` that would reach outside the served schema is refused.
- The default read tools: `pg_list_objects`, `pg_describe`, `pg_run_query`, `pg_count`, `pg_explain`, `pg_health`, and `pg_doctor`.
- Write and transaction tools that load in `write-only` and `read-write` modes.
- Five optional tool groups, loaded with `--tools`: `ddl`, `roles`, `maintenance`, `monitoring`, and `host`.
- Paged reads: 100 rows by default, up to 1,000 with `row_cap`, and a cursor for the next page.
- `json` and `jsonb` cells returned as JSON values, unless a number in them would lose precision or a key repeats.
- Transaction handles through `pg_transaction`. A handle rolls back after 60 seconds without a call (`handle_expiry_seconds`) or 300 seconds after it begins (`transaction_timeout_seconds`).
- A `COMMIT` cut off by a dropped connection reports whether the transaction committed or rolled back. `commit.outcome_unknown` comes back, with the transaction ID, only when the server gives no answer.
- Host-program tools that run the PostgreSQL programs on the host and write into `--output-dir`: `pg_dump`, `pg_dumpall_globals`, `pg_restore`, `pg_basebackup`, and `pg_upgrade_check`.
- `pg_restore` runs as one transaction by default.
- `pg_restore` adds `--if-exists` when `clean` is set, unless `if_exists` is `false`.
- `pg_dumpall_globals` leaves role password hashes out unless `no_role_passwords` is `false`.
- `pg_role` hashes a password with the server's `password_encryption` method before sending it.
- `pg_activity`, `pg_locks`, and `pg_top_queries` show `?` for every literal in statement text and withhold a statement that mentions a password, secret, or credential.
- `pg_list_objects` lists publications and event triggers, `pg_replication` reports what each publication covers, and `pg_privileges` lists default privileges with `list_defaults`.
- Resource templates `postgres://{database}/{schema}` and `postgres://{database}/{schema}/{table}`, listed 1,000 per page with a cursor.
- Three prompts: `diagnose_slow_query`, `review_indexes`, and `plan_column_change`.
- Three HTTP authentication modes: `none` (loopback only), bearer token, and OAuth, set with `--auth`.
- In OAuth mode, an outage of the key endpoint answers `503` with `Retry-After`.
- A per-caller HTTP rate limit of 300 requests a minute by default, set with `OWNPG_RATE_LIMIT_PER_MINUTE`, where `0` turns it off.
- `/healthz/live` and `/healthz/ready`, and `ownpg health`, which asks a running HTTP server whether it is ready and exits 0 or 1.
- A container image whose health check runs `ownpg health`.
- `docker-compose.yml`, an example deployment with PostgreSQL on an internal network and Docker secrets.
- OpenTelemetry metrics over OTLP/HTTP in HTTP mode when `OTEL_EXPORTER_OTLP_ENDPOINT` is set: `ownpg.calls`, `ownpg.refusals`, `ownpg.call.duration`, `ownpg.open_handles`, `ownpg.audit.dropped`, `ownpg.audit.degraded`, `ownpg.http.rejections`, `db.client.connection.count`, `db.client.connection.max`, and `db.client.connection.pending_requests`.
- `result_text` (`full` by default, or `summary`), which replaces the text copy of a long tool result with one line.
- A strict-role check that refuses to start on a superuser, an `rds_superuser` member, or a role with `BYPASSRLS`, on by default in HTTP mode and controlled with `--strict-role`.
- Connections over TCP, a Unix socket, or an SSH bastion host, with the in-process SSH client or the system `ssh` command selected by `--ssh-transport`.
- TLS with the libpq `sslmode` values and `--sslrootcert`, which also takes `system` for the platform trust store.
- An append-only, hash-chained audit log, on by default, checked with `ownpg audit verify`, and controlled with `--no-audit` and `--audit-path`.
- `audit_on_failure` (`refuse-writes` by default, or `refuse-all` or `continue`), which decides what runs while the audit log can't be written; a gap marker in the chain counts the lines that were lost.
- `audit_keep_days` (366 by default), which removes closed audit files of the same profile after that many days and records each removal in the chain.
- An audit line for every transaction handle the server closes on its own: after its idle expiry, at `transaction_timeout_seconds`, when its connection drops, and at shutdown.
- `ownpg doctor`, a connection and settings check with text or JSON output.
- `ownpg config`, connection-profile management with OS-keychain storage for passwords and SSH passphrases.
- `keychain_scope`: `file` (the default) binds a stored secret to the profile file and the server, and `target` binds it to the server alone.
- `ownpg man`, a manual page for the whole tool or one subcommand.
- `ownpg completions`, shell completion scripts for bash, elvish, fish, powershell, and zsh.
- `--log-file`, which also writes logs to a file rotated daily with the newest eight kept, and `--log-format text|json`.

[Unreleased]: https://github.com/devops-infinity/ownpg/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/devops-infinity/ownpg-releases/releases/tag/v0.1.1
[0.1.0]: https://github.com/devops-infinity/ownpg-releases/releases/tag/v0.1.0
