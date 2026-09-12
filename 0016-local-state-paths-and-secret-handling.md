# ADR-0016: Local state paths, file permissions, and secret handling

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

OwnPG keeps a profile file, an audit log, and an optional cache of catalog metadata. `reserve` derives branded config, cache, and data directories with `etcetera` (`context.rs:284-316`) and creates directories 0700 and files 0600; `owndbs` stores secrets in the OS keychain with `keyring`. Postgres MCP Pro's README warns that credentials passed through tool calls end up in chat history; the July 2026 audit found servers usable for port scanning through caller-supplied DSNs. The house rule forbids a secret on a command line or in a log. Where does OwnPG keep its state and its secrets?

## Decision Drivers

- The same branded path scheme as `reserve` (`devops.bd/ownpg`) so users find files where they expect.
- Secrets in files with 0600, in the environment, or in the OS keychain; never in tool arguments, URLs, or command lines.
- No data store beyond files; no embedded database.

## Options Considered

### Option A: etcetera paths, 0600 files, optional keychain, no secrets through tools

Directories follow the XDG layout on macOS and Linux and the roaming application data folder on Windows, always under `devops.bd/ownpg`. The config directory holds `profiles.toml` (0600, written atomically through a temporary file in the same directory, refused when its mode is looser, never tightened by OwnPG, and never replaced by a temporary-directory fallback); the data directory holds one `audit-<profile>-<pid>.jsonl` per process (0600) and, only for a multi-instance remote deployment, the elicitation state key; the cache directory holds an optional catalog cache with a TTL. Passwords come from the libpq password file, `PGPASSWORD`, a plain-text profile field (allowed at mode 0600 as the password file allows it, decided 2026-09-12), or the OS keychain through `keyring` when the user runs `ownpg config set-password <profile>`; SSH passphrases the same way. Tool arguments never accept a DSN, password, token, or key path; `config show` masks secrets; `doctor` reports file modes and refuses files that are too open.

- Pros: matches `reserve` and `owndbs` conventions; nothing new to learn.
- Pros: closes the tool-argument credential path by design.
- Cons: `keyring` adds a platform dependency (Security framework on macOS, Secret Service on Linux, Credential Manager on Windows).

### Option B: Secrets only in the environment

- Pros: simplest.
- Cons: `PGPASSWORD` is visible in the process list on some systems (the manual says so); client config files would carry it.

### Option C: An embedded database (SQLite) for audit and cache

- Pros: queryable.
- Cons: a data store the blueprint would then hold to migration and backup rules; JSON-lines and TOML are enough.

### Option D: Do nothing

- Cons: files would land in the working directory with default modes.

## Decision

We will store profiles, audit, and cache under the `devops.bd/ownpg` config, data, and cache directories with 0700 directories and 0600 files, accept secrets only from libpq sources, the profile, the environment, or the OS keychain, and never through tool arguments.

Option A won because it reuses the exemplar's conventions and removes the credential-in-chat-history failure entirely.

## Consequences

- Positive: `ownpg config path` prints where everything lives, and it is true (unlike `reserve` PART-1).
- Positive: a leaked audit log or profile carries no password.
- Negative: the keychain path needs a test on each platform.
- Neutral: the catalog cache is optional and off by default in 1.0.

## Reversibility

Cheap to reverse for the cache; expensive for the profile format once users have written profiles (a migration would be needed).

## Sources

- `/Users/sharkar/Git-Repositories/reserve/crates/reserve-cli/src/context.rs:284-316` and `files.rs:217-218`, `files.rs:484-488` (read 2026-09-12)
- `/Users/sharkar/Git-Repositories/owndbs/Cargo.toml` (`keyring`, `zeroize`, read 2026-09-12)
- https://www.postgresql.org/docs/18/libpq-envars.html (`PGPASSWORD` warning)
- https://github.com/crystaldba/postgres-mcp (README, credentials in chat history)
- `research/03-existing-postgresql-mcp-servers.md`, question 3 (DSN handling and the m10x.de audit)
