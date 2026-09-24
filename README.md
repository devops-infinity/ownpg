# OwnPG

PostgreSQL DBA tools for AI clients over the Model Context Protocol, scoped to one database and one schema.

- [What it does](#what-it-does)
- [Status](#status)
- [Prerequisites](#prerequisites)
- [Install](#install)
- [Quick start](#quick-start)
- [Connecting a client](#connecting-a-client)
- [Commands](#commands)
- [Configuration](#configuration)
- [Tool groups](#tool-groups)
- [Security](#security)
- [Support](#support)
- [Contributing](#contributing)
- [License](#license)

## What it does

OwnPG serves one PostgreSQL database and one schema to an MCP client, in read-only, write-only, or read-write mode. Every statement is parsed and classified before it runs, and the access mode decides what is allowed. A destructive statement asks for confirmation before it executes, and an append-only, hash-chained audit log records every call. OwnPG runs over stdio for a local MCP client such as Claude Desktop, or over Streamable HTTP for a remote MCP client, with bearer-token or OAuth authentication.

## Status

OwnPG is pre-release software at version 0.1.0. No version is tagged, and the crate is not published on crates.io. Releases, installation, and issue reporting for OwnPG happen in the public [devops-infinity/ownpg-releases](https://github.com/devops-infinity/ownpg-releases) repository.

## Prerequisites

- Rust 1.89 or newer to build from source. The workspace pins Rust 1.98.1 for its own build and test process.
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

`docker-compose.yml` in the repository root shows a full example deployment: a PostgreSQL service, Docker secrets for the database password and bearer tokens, and a read-only OwnPG service. The PostgreSQL data volume is external, so it survives `docker compose down -v`; create it once before the first start with `docker volume create ownpg-postgres-data`. The files under `secrets/` are mounted as they are, so they must be readable by user id 65532, the user the OwnPG image runs as.

## Quick start

Serve one database over stdio, read-only, with the default schema `public`:

```
ownpg serve -d app
```

Check the connection and settings before serving:

```
ownpg doctor -p staging --format json
```

Save a connection profile, and store its password in the OS keychain only if the database needs one:

```
ownpg config init
ownpg config set-password local
```

Serve over Streamable HTTP with bearer-token authentication:

```
OWNPG_BEARER_TOKENS="a-long-random-token read-write client-a" \
ownpg serve --http --auth bearer --bind 127.0.0.1:8765 -d app
```

## Connecting a client

The client starts and stops OwnPG for you; nothing runs until the client needs it, and nothing keeps running after the session ends. Register one entry per project database in the client's own MCP server list, with the connection given directly.

Claude Code reads a `.mcp.json` file in the project root, so each project names its own database:

```json
{
  "mcpServers": {
    "ownpg": {
      "command": "ownpg",
      "args": ["serve", "--database", "myproject_db", "--no-input"],
      "env": {
        "OWNPG_HOST": "127.0.0.1",
        "OWNPG_USER": "myproject_role"
      }
    }
  }
}
```

Add `"OWNPG_PASSWORD": "..."` to `env` only if the database actually needs one. A local PostgreSQL server using trust or peer authentication, the default for a superuser or your own OS user on a Homebrew or system install, needs no password at all: OwnPG connects the same way `psql` would, nothing to type, nothing to store.

Claude Desktop reads one file covering every project, so name each entry after the project it connects to. The file lives at `~/Library/Application Support/Claude/claude_desktop_config.json` on macOS, or `%APPDATA%\Claude\claude_desktop_config.json` on Windows. The same `command`/`args`/`env` block goes under a project-named key:

```json
{
  "mcpServers": {
    "ownpg-myproject": {
      "command": "ownpg",
      "args": ["serve", "--database", "myproject_db", "--no-input"],
      "env": {
        "OWNPG_HOST": "127.0.0.1",
        "OWNPG_USER": "myproject_role"
      }
    }
  }
}
```

A second project gets a second entry, `ownpg-otherproject`, pointed at a different database; each one launches as its own process, scoped to its own database.

If a password does need to go somewhere more permanent than a client config file, run `ownpg config init` once, then `ownpg config set-password <name>` to store it in the OS keychain, and reference `--profile <name>` instead of the raw connection flags. A keychain entry belongs to the profile file and to the user, host, and port it was saved for, so another profile file never receives it. To share one entry across profile files that point at the same server, set `keychain_scope = "target"` (or `OWNPG_KEYCHAIN_SCOPE=target`). That's a convenience for a password you'd rather not retype or paste, not a required step; the plain `env` block above works with nothing set up beforehand. This repository also carries `mcpb/manifest.json`, the manifest for Claude Desktop's packaged extension format.

### Connecting over Streamable HTTP

A remote MCP client, or a client that cannot spawn a local process, connects to a running `ownpg serve --http` instance instead of launching its own. Point the client at `http://<bind-address>/mcp` (the path is always `/mcp`, whatever `--bind` address you chose), and set `--auth` to match how the client authenticates:

- `--auth none`: no credential, only usable when `--bind` stays on the loopback interface.
- `--auth bearer`: the client sends `Authorization: Bearer <token>` on every request, checked against the tokens set with `OWNPG_BEARER_TOKENS` or `OWNPG_BEARER_TOKENS_FILE`.
- `--auth oauth`: the client sends a JWT issued by an OAuth authorization server, verified against that server's published JSON Web Key Set.

A client that supports MCP's OAuth-protected-resource discovery reads `/.well-known/oauth-protected-resource/mcp` on the same host and port to find the authorization requirements automatically.

Each caller can make 300 requests a minute by default. `OWNPG_RATE_LIMIT_PER_MINUTE` changes the limit, and `0` turns it off.

`/healthz/live` answers as long as the process runs. `/healthz/ready` answers `200` only while PostgreSQL is reachable, the OAuth key endpoint answers when `--auth oauth` is set, and, under `audit_on_failure = refuse-all`, the audit log can be written. `ownpg health` asks the second one and exits 0 or 1, so a container health check needs no `curl`; the published image uses it.

## Commands

- `ownpg serve`: serve the MCP protocol. The default command when no subcommand is given.
- `ownpg doctor`: check the connection and report the result as text or JSON.
- `ownpg health`: ask a running `ownpg serve --http` whether it is ready. It exits 0 when it is and 1 when it isn't; `--live` only checks that the process answers.
- `ownpg config`: manage connection profiles and local state (`show`, `path`, `init`, `set-password`, `unset-password`, `set-ssh-passphrase`, `unset-ssh-passphrase`).
- `ownpg audit verify`: check that every line of an audit log chains to the one before it.
- `ownpg man`: print a manual page, for the whole tool or one named subcommand.
- `ownpg completions`: print a shell completion script for bash, elvish, fish, powershell, or zsh.

Run `ownpg man` for the full reference to every command, flag, and environment variable.

## Configuration

OwnPG resolves its settings in this order: command-line flags, environment variables, a connection profile file, libpq-standard environment variables and files (`PGHOST`, `PGUSER`, `PGDATABASE`, `PGSERVICE`, `PGSERVICEFILE`, `PGPASSFILE`), and built-in defaults.

The connection takes a host, a port (default 5432), a user, and a password, which can come from a flag, an environment variable, a `.pgpass` file, or the OS keychain. A libpq connection string can use any libpq keyword. OwnPG applies the ones it supports, warns about the ones it ignores, and refuses the ones that ask for a guarantee it can't give, such as `require_auth` or `gssencmode=require`. `--database` is required, `--schema` defaults to `public`, and `--mode` defaults to `read-only`. A database behind a bastion host is reachable with `--ssh`, using an in-process SSH client or the system `ssh` command.

A transaction handle is rolled back after `handle_expiry_seconds` (60 by default) with no call, or `transaction_timeout_seconds` (300 by default) after it began, whichever comes first. If the connection drops while `COMMIT` is running, OwnPG asks the server whether that transaction committed and reports the answer; only a check that gets no answer comes back as `commit.outcome_unknown`, with the transaction ID to look up.

Each tool result carries its data twice by default: once as `structuredContent` and once as text, as the MCP specification asks. Gemini CLI and Cursor read only the text, so they need it. OpenAI's documentation says ChatGPT reads both, so it gets every row twice. On a server whose clients all read `structuredContent`, set `result_text = "summary"` (or `OWNPG_RESULT_TEXT=summary`) and the text shrinks to one line naming the row count and the next-page cursor. A text of 400 bytes or less is sent as it is either way.

The audit log is on by default. While it can't be written, OwnPG refuses the tools that can change the database and keeps reads working; set `audit_on_failure` to `refuse-all` or `continue` to change that. Closed audit files older than 366 days are removed; `audit_keep_days` sets another number of days, and `0` keeps them forever.

A connection profile is a TOML file with one `[profiles.<name>]` table per named connection, covering the mode, database, schema, connection string, TLS settings, timeouts, row and byte caps, the strict-role check, the extra tool groups to load, the audit log settings, and the SSH settings for that profile. `ownpg config init` writes a starter profile file naming every key. `ownpg config show` prints the resolved settings for a profile with secrets masked.

Run `ownpg config init --dry-run` or `ownpg man` for the complete list of environment variables and profile keys.

To remove OwnPG from a machine, run `ownpg config unset-password <name>` and `ownpg config unset-ssh-passphrase <name>` for each profile that stored a secret, then delete the binary and the profile file, data directory, and log directory that `ownpg config path` prints. The data directory holds the audit logs, so copy them elsewhere first if you need to keep them.

## Tool groups

`objects`, `read`, and `health` load by default. `write` and `transactions` load automatically once the access mode allows writes (`write-only` or `read-write`). `--tools` (or `OWNPG_TOOLS`) adds any of `ddl`, `roles`, `maintenance`, `monitoring`, `host`. A tool group not allowed under the active access mode is refused at startup.

## Security

A destructive statement runs only after an explicit `confirm: true` argument or an elicitation round trip sealed to that one statement, the caller, and the database it was asked for. A `DROP ... CASCADE` lists every object it would also drop, and it is refused when any of them lies outside the served schema. See [SECURITY.md](SECURITY.md) for the vulnerability-reporting process and the data OwnPG handles.

## Support

File an issue at [devops-infinity/ownpg-releases](https://github.com/devops-infinity/ownpg-releases/issues/new).

## Contributing

File an issue at [devops-infinity/ownpg-releases](https://github.com/devops-infinity/ownpg-releases/issues/new) to report a bug or request a change.

## License

OwnPG is dual-licensed under the MIT license ([LICENSE-MIT](LICENSE-MIT)) and the Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE)). You may choose either.
