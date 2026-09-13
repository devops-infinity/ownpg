# OwnPG agent instructions

Instructions for an AI agent working in this repository. `CLAUDE.md` and `AGENTS.md` carry identical text under two names, so every tool finds the rules under the name it looks for. Change one and copy it to the other in the same run.

## Precedence

The Claude Code global constitution at `~/.claude/skills/constitution/constitution.md` (mirrored at `~/.agents/skills/constitution/constitution.md` for Codex) outranks this file and every convention in it. This repository has no project-specific constitution at `.specify/memory/constitution.md` or a root `constitution.md`.

## Architecture

The workspace has two crates. `ownpg-core` holds every piece of business and protocol logic: the statement classifier, the engine, the MCP (Model Context Protocol) server for stdio and Streamable HTTP, the tool implementations, connection handling (TLS, SSH), and configuration resolution. `ownpg` is a thin CLI shell: argument parsing, subcommand dispatch, output formatting, logging setup, and manual-page and completion generation. `ownpg` depends on `ownpg-core`; the dependency never runs the other way. Engine, tool, classifier, connection, and protocol logic belongs in `crates/ownpg-core/src/`, never in `crates/ownpg/src/`.

See `README.md` for what the project does and how to run it, `CHANGELOG.md` for the version history, and `SECURITY.md` for the vulnerability-reporting process.

## Conventions this repository enforces

- `cargo fmt --all -- --check`: formatting.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: lint, denying every warning.
- `cargo nextest run --workspace --all-features --locked`: unit and integration tests. `cargo test --workspace --all-features --locked --doc`: doc tests.
- `cargo doc --workspace --all-features --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`: the documentation build stays clean.
- `cargo machete`: no unused dependencies.
- `cargo llvm-cov nextest --workspace --locked --fail-under-lines 80`: line coverage stays at 80 percent or higher.
- `cargo audit --deny warnings` and `cargo deny check`: dependency advisories, license, and source checks against `deny.toml`.
- `cargo semver-checks check-release -p ownpg-core`: the public API of `ownpg-core` stays compatible with the published crate.
- Every tracked file under `crates/`, `tools/`, and the repository root's markdown files is grepped for the em-dash character, and the same scope excluding `LICENSE-*` is grepped for a short list of words that flag borrowed or superseded code. See the house-rules job in `.github/workflows/ci.yml` for the exact pattern.
- No markdown table appears in any tracked markdown file at the repository root.
- Every GitHub Actions step is pinned to a full commit SHA, never a floating tag.

## Do

- Keep business and protocol logic in `ownpg-core`. Keep `ownpg` limited to the CLI shell.
- Run `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo nextest run` before treating a change as finished.
- Verify a claim about behavior against the actual source in `crates/`, not against a commit message or an earlier version of a document.

## Don't

- Don't write `unsafe` code. The workspace denies it (`unsafe_code = "forbid"`).
- Don't use `unwrap()`, `expect()`, `panic!()`, indexing or slicing, integer division, `todo!()`, `unimplemented!()`, `dbg!()`, `println!()`, or `eprintln!()` outside test code. Clippy denies each one.
- Don't add `anyhow`, `openssl`, `openssl-sys`, `native-tls`, `atty`, `ansi_term`, `structopt`, `backoff`, `lazy_static`, or `exitcode` as a dependency. `deny.toml` bans each one by name.
- Don't pin a GitHub Actions step to a tag or a branch. Pin the full commit SHA.
- Don't restate installation or usage instructions here. `README.md` covers them.
