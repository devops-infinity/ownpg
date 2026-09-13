# OwnPG

PostgreSQL DBA tools for AI clients over the Model Context Protocol, scoped to one database and one schema.

## What it does

OwnPG serves one PostgreSQL database and one schema to an MCP client, in read-only, write-only, or read-write mode. Every statement is parsed and classified before it runs, and the access mode decides what is allowed. A destructive statement asks for confirmation before it executes, and an append-only, hash-chained audit log records every call. OwnPG runs over stdio for a local MCP client such as Claude Desktop, or over Streamable HTTP for a remote MCP client, with bearer-token or OAuth authentication.

## Status

OwnPG is pre-release software at version 0.1.0. No version is tagged, and the crate is not published on crates.io. Releases, installation, and issue reporting for OwnPG happen in the public [devops-infinity/ownpg-releases](https://github.com/devops-infinity/ownpg-releases) repository.

## Prerequisites

- Rust 1.89 or newer to build from source. The workspace pins Rust 1.97.1 for its own build and test process.
- A reachable PostgreSQL server, version 14 through 18.
- For the host-program tools (`pg_dump`, `pg_dumpall`, `pg_restore`, `pg_basebackup`, `pg_upgrade --check`), the matching PostgreSQL client programs on the machine running OwnPG.

## Install

Clone the repository and install the `ownpg` binary from a checkout:

```
cargo install --path crates/ownpg --locked
```

Or build the container image from the Dockerfile in the repository root:

```
docker build -t ownpg .
```

`docker-compose.yml` in the repository root shows a full example deployment: a PostgreSQL service, Docker secrets for the database password and bearer tokens, and a read-only OwnPG service.

## Quick start

Serve one database over stdio, read-only, with the default schema `public`:

```
ownpg serve -d app
```

Check the connection and settings before serving:

```
ownpg doctor -p staging --format json
```

Save a connection profile and store its password in the OS keychain:

```
ownpg config init && ownpg config set-password local
```

Serve over Streamable HTTP with bearer-token authentication:

```
ownpg serve --http --auth bearer --bind 127.0.0.1:8765 -d app
```

## Commands

- `ownpg serve`: serve the MCP protocol. The default command when no subcommand is given.
- `ownpg doctor`: check the connection and report the result as text or JSON.
- `ownpg config`: manage connection profiles and local state (`show`, `path`, `init`, `set-password`, `unset-password`, `set-ssh-passphrase`, `unset-ssh-passphrase`, `cache-clear`).
- `ownpg audit verify`: check that every line of an audit log chains to the one before it.
- `ownpg man`: print a manual page, for the whole tool or one named subcommand.
- `ownpg completions`: print a shell completion script for bash, elvish, fish, powershell, or zsh.

Run `ownpg man` for the full reference to every command, flag, and environment variable.

## Configuration

OwnPG resolves its settings in this order: command-line flags, environment variables, a connection profile file, libpq-standard environment variables and files (`PGHOST`, `PGUSER`, `PGDATABASE`, `PGSERVICE`, `PGSERVICEFILE`, `PGPASSFILE`), and built-in defaults.

The connection takes a host, a port (default 5432), a user, and a password, which can come from a flag, an environment variable, a `.pgpass` file, or the OS keychain. `--database` is required, `--schema` defaults to `public`, and `--mode` defaults to `read-only`. A database behind a bastion host is reachable with `--ssh`, using an in-process SSH client or the system `ssh` command.

A connection profile is a TOML file with one `[profiles.<name>]` table per named connection, covering the mode, database, schema, connection string, TLS settings, timeouts, row and byte caps, the strict-role check, the extra tool groups to load, the audit log settings, and the SSH settings for that profile. `ownpg config init` writes a starter profile file naming every key. `ownpg config show` prints the resolved settings for a profile with secrets masked.

Run `ownpg config init --dry-run` or `ownpg man` for the complete list of environment variables and profile keys.

## Tool groups

`objects`, `read`, and `health` load by default. `--tools` (or `OWNPG_TOOLS`) adds any of: `write`, `transactions`, `ddl`, `roles`, `maintenance`, `monitoring`, `host`. A tool group not allowed under the active access mode is refused at startup.

## Security

A destructive statement runs only after an explicit `confirm: true` argument or an elicitation round trip sealed to that one statement. See [SECURITY.md](SECURITY.md) for the vulnerability-reporting process and the data OwnPG handles.

## Support

File an issue at [devops-infinity/ownpg-releases](https://github.com/devops-infinity/ownpg-releases/issues/new).

## License

OwnPG is dual-licensed under the MIT license ([LICENSE-MIT](LICENSE-MIT)) and the Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE)). You may choose either.
