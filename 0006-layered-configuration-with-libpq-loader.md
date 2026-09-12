# ADR-0006: Layered configuration with a TOML profile file and an in-house libpq loader

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The brief asks for full flexibility: local PostgreSQL with the platform's default credentials, with or without a password, across Homebrew, Docker, Linux packages, and Postgres.app, plus SSH. psql users expect `PGHOST`, the password file, and the service file to work. `tokio-postgres` parses connection strings but reads none of those (issues #654 and #729 open since 2020 and 2021), and the third-party loaders are stale (`pg-client-config` 2023) or AGPL. `reserve` advertises a config directory but never built a file layer. What is the configuration model?

## Decision Drivers

- Zero configuration on a default local install.
- The same knowledge psql has: environment, password file, service file, with libpq's precedence.
- One documented order that `config show` can print with origins, as `reserve` does for flags and environment.
- Secrets never on the command line and never in logs.

## Options Considered

### Option A: Five layers, resolved once, with an in-house libpq loader

Order, later wins: built-in platform presets; libpq sources (connection string over service file over `PG*` environment over defaults, as the manual states); the OwnPG TOML profile file (0600 and refused when looser, under the OwnPG config directory, `format = 1` at the top, `[profiles.<name>]` sections, unknown keys refused with the file, line, and key named, a plain-text `password` allowed as the libpq password file allows it, or a keychain reference instead; `--config <file>` and `OWNPG_CONFIG` replace the search, and there is no project-local config file); `OWNPG_*` environment variables; command-line flags. The libpq loader is a few hundred lines written against the PostgreSQL 18 manual: environment variables, the password file with the 0600 rule on every Unix, the service files with the user file winning on a duplicate name.

- Pros: psql habits work unchanged; OwnPG-specific settings (mode, schema, SSH, caps) layer on top.
- Pros: `Origins` from `reserve` extends to `file` and `libpq` as two more origins.
- Cons: the loader is OwnPG's code to test and maintain.

### Option B: Connection string only, as most surveyed servers do

- Pros: nothing to write.
- Cons: no password file, no service file, no presets; users paste passwords into client config files.

### Option C: Depend on `sqlx` for its built-in environment and password-file support

- Pros: exists today.
- Cons: `sqlx` is rejected as the driver (ADR-0007); its password-file permission check runs only on Linux; no service file.

### Option D: Do nothing (flags only)

- Cons: every client config would carry the whole DSN.

## Decision

We will resolve configuration in five layers (presets, libpq sources, TOML profile, `OWNPG_*` environment, flags) in one function, with an in-house libpq loader that follows the PostgreSQL 18 manual, and print every setting with its origin in `config show`.

Option A won because it gives psql users what they expect and gives `config show` a truthful answer, at the cost of a small, well-specified loader.

## Consequences

- Positive: `ownpg serve --database app` works on a default Homebrew install with nothing else.
- Positive: SSH and mode settings live in a profile, so client configs stay short.
- Negative: the password-file and service-file parsers need their own tests, including the permission rule on macOS.
- Neutral: `PGPASSWORD` is honored but discouraged in the docs, as the manual does.
- Neutral: the database name resolves through the same layers (`--database`, `OWNPG_DATABASE`, the profile, `PGDATABASE`) and must resolve before `serve` starts.

## Reversibility

Cheap to reverse for the profile layer; expensive for the libpq loader once users rely on it. Revisit if `tokio-postgres` adds native support (issues #654 and #729).

## Sources

- https://www.postgresql.org/docs/18/libpq-connect.html, libpq-envars, libpq-pgpass, libpq-pgservice (PostgreSQL 18.6)
- https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html (0.7.18)
- https://github.com/rust-postgres/rust-postgres/issues/654 and /issues/729
- `research/04-rust-postgresql-connectivity-and-ssh.md`, findings 12 to 17
- `/Users/sharkar/Git-Repositories/reserve/crates/reserve-cli/src/cli.rs:8-155` (the `Origins` layer), `research/07-reserve-reuse-map.md` PART-1
