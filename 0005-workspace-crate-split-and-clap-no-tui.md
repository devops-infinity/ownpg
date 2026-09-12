# ADR-0005: Workspace with a core crate and a thin CLI crate, clap derive, no terminal interface

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The `rust-cli-apps` blueprint asks for a recorded crate shape, argument parser, and interface kind, and says to mirror an exemplar when one exists. `reserve` is a three-member workspace (`crates/reserve-core`, `crates/reserve-cli`, `crates/reserve-verify`) with clap 4 derive (`env`, `wrap_help`), `clap_complete`, `clap_mangen`, a ratatui picker for its own product need, and a lint deny-list in `[workspace.lints]`. `owndbs` uses `crates/core`, `crates/cli`, `crates/tui`, `crates/gui`. What shape does OwnPG take?

## Decision Drivers

- Mirror the exemplar so tooling, CI, and release scripts transfer with minimal edits.
- A library core that integration tests can drive without a subprocess.
- No interface OwnPG does not need: an MCP server is headless.
- Names must not collide with the crate on crates.io (`ownpg` is free).

## Options Considered

### Option A: `crates/ownpg-core` plus `crates/ownpg` (binary), clap derive, no TUI

The core holds configuration, connection, SSH, classification, catalog queries, tool handlers, and the MCP server wiring; the binary parses arguments, builds the runtime context, calls the core, and maps the result to an exit class. Optional third member later: an unpublished `ownpg-verify` smoke crate, as in `reserve`.

- Pros: identical to the `reserve` shape, so `tools/release.sh` publishes core then binary unchanged in structure.
- Pros: `assert_cmd` tests and in-process rmcp tests both have a target.
- Cons: two crates to version together.

### Option B: One crate with `lib.rs` and `main.rs`

- Pros: simpler manifest.
- Cons: breaks the exemplar's release flow and the blueprint's crate-split item; harder to add a verify crate later.

### Option C: Add a ratatui interface for interactive browsing

- Pros: a human-facing view.
- Cons: out of scope (PRD section 4); doubles the terminal lifecycle surface; nothing in the brief asks for it.

### Option D: Do nothing (let `cargo new` decide)

- Cons: a single binary crate with no library, which the blueprint rejects.

## Decision

We will create a workspace with `crates/ownpg-core` (library) and `crates/ownpg` (binary named `ownpg`), parse the command surface once with clap derive, generate man pages and completions from the same definition, and ship no terminal interface.

Option A won because it is the proven exemplar shape and it gives every test a direct entry point.

## Consequences

- Positive: `reserve`'s `Cargo.toml` workspace block, lint deny-list, profiles, `build.rs`, `cli.rs` structure, `main.rs`, and `context.rs` copy over with renames.
- Positive: `rust-version = "1.89"` and edition 2024 are set once in `[workspace.package]`.
- Negative: any future GUI or TUI is a new crate and a new ADR.
- Neutral: the binary name is set explicitly, not inherited from the package name.
- Neutral: the core and the binary share one error type (`Error` carrying `ExitClass` and `ErrorId`), so `anyhow` is not a dependency.

## Reversibility

Expensive to reverse once handlers and tests exist against the core crate. Revisit only if a second front end appears.

## Sources

- `~/.claude/skills/rust-cli-apps/SKILL.md`, "The application baseline"
- `/Users/sharkar/Git-Repositories/reserve/Cargo.toml:1-129`, `crates/reserve-cli/src/cli.rs:215-365`, `build.rs:17-60` (read 2026-09-12)
- `/Users/sharkar/Git-Repositories/owndbs/Cargo.toml:1-60` (read 2026-09-12)
- `research/07-reserve-reuse-map.md`, MAP-1 and MAP-2
