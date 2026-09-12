# ADR-0008: In-process SSH tunnels with russh, and the system ssh as an optional mode

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The brief requires PostgreSQL behind SSH. Among the surveyed MCP servers only DBHub offers SSH. In Rust the choices are `russh` 0.63.3 (2026-09-09, Apache-2.0, MSRV 1.89, 6.84 million downloads, in-process), `openssh` 0.11.6 (spawns the system `ssh` under ControlMaster), `ssh2` (libssh2, synchronous, OpenSSL), and `thrussh` (publishing, but its repository link is dead and its README says agent and encrypted-key support are missing). Shipping Rust PostgreSQL tools (`rpg`, `ferox`) use `russh`. How does OwnPG reach a database behind a bastion?

## Decision Drivers

- No local port to open and no external process on the default path.
- Key files with a passphrase, the SSH agent, password auth, and client-config aliases.
- Host key verification that refuses unknown keys by default (OpenSSH's default is ask, never silently accept).
- ProxyJump chains for real bastion setups.
- Works inside a container image that has no `ssh` binary.

## Options Considered

### Option A: russh in-process, system ssh as `ssh_mode = "system"`

Connect with `russh::client::connect`, authenticate (publickey through `keys::load_secret_key` with passphrase, agent through `keys::agent::client::AgentClient` over `SSH_AUTH_SOCK`, password, keyboard-interactive), open `channel_open_direct_tcpip(db_host, db_port, ...)`, turn it into a stream with `into_stream()`, and pass it to `tokio_postgres::Config::connect_raw`. Apply a 10 second connect timeout. Verify host keys in `Handler::check_server_key` against the known-hosts file with `keys::known_hosts::check_known_hosts_path`; refuse unknown keys unless `--ssh-trust-new-host` is set, and print the fingerprint. Parse client-config aliases with `russh-config` 0.58.0; build ProxyJump chains with `client::connect_stream` over the previous hop. Offer `ssh_mode = "system"` (the `openssh` crate) for setups that need `Match` blocks, security keys, or other OpenSSH-only features.

- Pros: no port, no process, works in containers, TLS inside the tunnel still possible.
- Pros: proven by `rpg` and `ferox`.
- Cons: raises the MSRV to 1.89; host-key and config parsing are OwnPG's responsibility; `russh-config` keyword coverage must be read from source.

### Option B: openssh (system ssh) only

- Pros: every OpenSSH config feature for free.
- Cons: needs an `ssh` binary, spawns a process, and forwards a local port (no stream injection); fails in minimal containers.

### Option C: ssh2 (libssh2)

- Pros: mature C library.
- Cons: synchronous, links OpenSSL next to rustls, no async stream for `connect_raw`.

### Option D: Do nothing (document `ssh -L` for users)

- Cons: the brief requires SSH support; users would keep manual port forwards open.

## Decision

We will open SSH tunnels in-process with `russh`, verify host keys against the known-hosts file with refusal as the default, support key, agent, and password auth, parse client-config aliases, build ProxyJump chains, and offer the system `ssh` as an opt-in mode.

Option A won because it needs no port, no process, and no binary, and because two shipping Rust PostgreSQL tools already prove the pattern.

## Consequences

- Positive: `--ssh user@bastion` is the whole user experience; the database connection rides a direct-tcpip channel.
- Positive: the remote multi-client mode can open one channel per pooled connection through `Manager::from_connect`.
- Negative: `rust-version = "1.89"` for the workspace.
- Negative: known-hosts parsing, agent protocol, and config parsing need integration tests; they run against an in-process `russh` server that each test starts, so no `sshd` and no container is needed on any machine.
- Neutral: `thrussh` and `ssh2` are not used.

## Reversibility

Expensive to reverse once the tunnel is wired into the connector; the `system` mode is the escape hatch. Revisit if `russh` stops releasing or if a tokio-native OpenSSH-config crate reaches full coverage.

## Sources

- https://crates.io/api/v1/crates/russh (0.63.3, read 2026-09-12)
- https://docs.rs/russh/latest/russh/struct.Channel.html, https://docs.rs/russh/latest/russh/client/trait.Handler.html, https://docs.rs/russh/latest/russh/keys/index.html
- https://docs.rs/russh-config/latest/russh_config/ (0.58.0)
- https://docs.rs/openssh/latest/openssh/ (0.11.6)
- https://raw.githubusercontent.com/NikolayS/rpg/main/Cargo.toml and https://github.com/NikolayS/rpg/issues/824 (host-key default)
- `research/04-rust-postgresql-connectivity-and-ssh.md`, findings 33 to 42
