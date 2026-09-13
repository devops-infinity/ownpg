# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/).

## [Unreleased]

### Added

- An MCP (Model Context Protocol) server that serves one PostgreSQL database and one schema, over stdio by default or over Streamable HTTP with `--http`.
- Three access modes: read-only, write-only, and read-write, set with `--mode`.
- Three HTTP authentication modes: none (loopback only), bearer token, and OAuth, set with `--auth`.
- Seven optional tool groups beyond the default read and health tools: write, transactions, DDL, roles, maintenance, monitoring, and host programs, loaded with `--tools`.
- A strict-role check that refuses to start on a superuser or a role with `BYPASSRLS`, on by default in HTTP mode, controlled with `--strict-role`.
- Connectivity to PostgreSQL over TCP, a Unix socket, or an SSH bastion host, with an in-process SSH client or the system `ssh` command selectable with `--ssh-transport`.
- An append-only, hash-chained audit log, on by default, verified with `ownpg audit verify`, controlled with `--audit` and `--audit-path`.
- `ownpg doctor`, a connection and settings check with text or JSON output.
- `ownpg config`, connection-profile management with OS-keychain-backed password and SSH-passphrase storage.
- `ownpg man`, a generated manual page for the whole tool or one subcommand.
- `ownpg completions`, shell completion scripts for bash, elvish, fish, powershell, and zsh.
- Daily log-file rotation, keeping the newest eight files, with `--log-format text|json`.
