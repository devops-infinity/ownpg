# OwnPG agent instructions

Instructions for an AI agent working in this repository. `CLAUDE.md` and `AGENTS.md` carry identical text under two names, so every tool finds the rules under the name it looks for. Change one and copy it to the other in the same run.

## Architecture

The workspace has two crates. `ownpg-core` holds every piece of business and protocol logic: the statement classifier, the engine, the MCP (Model Context Protocol) server for stdio and Streamable HTTP, the tool implementations, connection handling (TLS, SSH), and configuration resolution. `ownpg` is a thin CLI shell: argument parsing, subcommand dispatch, output formatting, logging setup, and manual-page and completion generation. `ownpg` depends on `ownpg-core`; the dependency never runs the other way. Engine, tool, classifier, connection, and protocol logic belongs in `crates/ownpg-core/src/`, never in `crates/ownpg/src/`.

See `README.md` for what the project does and how to run it, `CHANGELOG.md` for the version history, `SECURITY.md` for the vulnerability-reporting process, and `RELEASING.md` for the release checklist.

## Local verification

This repository enforces its conventions locally, not in a hosted CI service. Run `tools/install-hooks.sh` once per checkout to point git at `.githooks/`: `.githooks/pre-commit` scans staged changes for secrets with `gitleaks`, and `.githooks/pre-push` runs `tools/verify.sh`, the full gate, before a push is allowed to leave the machine. Run `tools/verify.sh` directly at any time to check the same gate without pushing.

## Conventions this repository enforces

- `cargo fmt --all -- --check`: formatting.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: lint, denying every warning.
- `cargo nextest run --workspace --all-features --locked`: unit and integration tests. `cargo test --workspace --all-features --locked --doc`: doc tests. The live database tests need `OWNPG_TEST_DSN`, a libpq connection string or URI for a role that can create databases; `tools/verify.sh` stops without it unless `OWNPG_SKIP_LIVE_TESTS=1` is set, so a run never passes with the live tests silently skipped.
- `cargo doc --workspace --all-features --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`: the documentation build stays clean.
- `cargo machete`: no unused dependencies.
- `cargo audit --deny warnings` and `cargo deny check`: dependency advisories, license, and source checks against `deny.toml`.
- `bash -n` on every tracked shell script, then `shellcheck -x -P .` and `shfmt -d` on `tools/*.sh` and `.githooks/pre-commit`/`.githooks/pre-push`: every shell script parses, lints clean, and stays formatted. `shellcheck`, `shfmt`, and `bats` must be installed; the gate stops when one is missing. The scripts need bash 4.4 or newer and hand themselves to a newer bash from Homebrew or `/usr/local` when an older one starts them. `bats tools/tests/toolset.bats` covers `tools/lib.sh`, `tools/install-hooks.sh`, both git hooks, and the comment-detection pattern this file's own gate uses.
- Every tracked file under `crates/`, `tools/`, and the repository root's markdown files is grepped for the em-dash character, and the same scope excluding `LICENSE-*` is grepped for a short list of words that flag borrowed or superseded code. Every tracked `.rs` file is grepped for a code comment, and every tracked shell, TOML, YAML, and Docker file for a `#` comment line. See `tools/verify.sh` for the exact patterns.
- The repository root's markdown files are grepped against the AI-footprint word list at `$HOME/.claude/rules/banned-words.md`, when that file exists on the machine running the gate; a checkout without it skips this one check with a warning rather than failing.
- No markdown table appears in any tracked markdown file at the repository root.
- `cargo llvm-cov nextest --workspace --locked --fail-under-lines 80` (line coverage) and `cargo semver-checks check-release -p ownpg-core` (public-API compatibility with the published crate) are run by hand periodically; neither is part of `tools/verify.sh`, since coverage instrumentation recompiles the whole workspace and semver-checks needs the registry to compare against.

## Do

- Keep business and protocol logic in `ownpg-core`. Keep `ownpg` limited to the CLI shell.
- Run `tools/verify.sh` before treating a change as finished, or let `.githooks/pre-push` run it for you.
- Verify a claim about behavior against the actual source in `crates/`, not against a commit message or an earlier version of a document.

## Don't

- Don't write `unsafe` code. The workspace denies it (`unsafe_code = "forbid"`).
- Don't use `unwrap()`, `expect()`, `panic!()`, indexing or slicing, integer division, `todo!()`, `unimplemented!()`, `dbg!()`, `println!()`, or `eprintln!()` outside test code. Clippy denies each one.
- Don't write a code comment anywhere, in any file, for any reason. `tools/verify.sh` refuses one on sight.
- Don't add `anyhow`, `openssl`, `openssl-sys`, `native-tls`, `atty`, `ansi_term`, `structopt`, `backoff`, `lazy_static`, or `exitcode` as a dependency. `deny.toml` bans each one by name.
- Don't add a hosted CI workflow. This repository verifies itself locally, on the machine making the change.
- Don't restate installation or usage instructions here. `README.md` covers them.
