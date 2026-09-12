# ownpg - project rules

Binding instructions for any AI agent working in this repository. `CLAUDE.md` and `AGENTS.md` carry identical text under two names, so every tool finds the rules under the name it looks for; change one and copy it to the other in the same run. The Principal Architect's live instruction outranks everything here. Below that: the global constitution, at `~/.claude/skills/constitution/constitution.md` for Claude Code and mirrored at `~/.agents/skills/constitution/constitution.md` for Codex; then this repository's own constitution, at `constitution.md` in the repository root; then this file; then skills, agents, and commands.

## What this is

A Rust server that gives an AI client one PostgreSQL database and one schema over the Model Context Protocol, in read-only, write-only, or read-write mode, over stdio first and Streamable HTTP second. Two crates in one workspace:

- `crates/ownpg-core`: all the logic. Configuration, connection, statement classification, SQL rendering, result shaping, the audit sink, the tool handlers, the protocol server. Its `config`, `classify`, `render`, `shape`, and `audit` modules import neither the protocol crate nor the driver crate.
- `crates/ownpg`: a thin binary named `ownpg`. Parses the command surface, builds a context, calls the core, turns the result into an exit code.

Source repository `devops-infinity/ownpg` (private). Public release artifacts and bug reports, no source, live at `devops-infinity/ownpg-releases`; the crate's `repository` field in `Cargo.toml` points there on purpose, since that is what the installer scripts, the Homebrew formula, the npm package, and `cargo binstall` resolve against. Published to crates.io only when the Principal Architect says so.

The planning pack (the research report, the PRD, the plan, and the research notes) lives under `docs/planning/`, and the decision records live at the repository root as `NNNN-*.md`. Read the PRD and the plan before any phase work; the plan says which phase is open and what its exit gate is.

See `README.md` for the command surface and `SECURITY.md` for the reporting process and the threat model; this file does not repeat either.

## This is our own product (MANDATORY)

`ownpg` was written from scratch. Other servers in this space were read during planning for ideas about the problem; nothing of them is carried here and nothing may be added.

- Never name, cite, link, or hint at any other project, its author, or its repository, anywhere in the product: source, tests, data, `README.md`, `CHANGELOG.md`, `SECURITY.md`, `docs/reference`, `docs/guides`, commit messages, or output. The planning pack under `docs/planning/` and the decision records keep their citations as internal evidence; nothing from them is quoted into the product.
- Never copy a flag name, a subcommand, a tool name, an error message, a status word, or an argument name from another tool. Where an industry-standard word exists (`--database`, `--schema`, `--json`, `--timeout`) use it because it is standard, not because another tool used it.
- PostgreSQL's own names and the Model Context Protocol's own names are protocol facts. They stay.
- Never write compatibility or transition code: no shim, no wrapper for an old caller, no re-export alias, no dual path, no stub. Replace the old form outright and delete it. The one exception is the two-minor-release tool deprecation window in ADR-0019.

Audit before any release:

```sh
git ls-files -- crates tools docs ':(top,glob)*.md' | grep -vE '^docs/planning/|^[0-9]{4}-.*\.md$' |
  xargs grep -niE 'legacy|backward.compat|inspired by|based on|ported from|fork of'
```

## Comments

None. Not one line. A name that says what it does replaces a comment explaining it, and a test that shows why a guard exists replaces a comment defending it. Help text for a flag goes through `#[arg(help = ...)]` and `#[command(about = ...)]`, never through a doc comment. Before calling a file done, confirm it carries no `//`, `///`, or `//!` line:

```sh
rg -n '^\s*(//|///|//!)' crates/ && echo "comments found" || echo "no comments"
```

## Rebuilding for a test run

Always go through the lifecycle script rather than a bare `cargo build`, because a stale binary on `PATH` produces wrong results:

```sh
./tools/reinstall.sh                 # clean, gate, build release, install, verify
./tools/reinstall.sh --debug         # faster build when iterating
./tools/reinstall.sh --skip-gate     # only when the gate just passed
./tools/reinstall.sh --keep-cache    # leave the ownpg cache directory in place
./tools/reinstall.sh --full-clean    # empty target/ and pay for one cold build
./tools/reinstall.sh --keep-build    # leave target/ alone this run
./tools/reinstall.sh --stale-days 14 # drop unused build data older than this many days (default 7)
./tools/reinstall.sh --dir /usr/local/bin
```

It removes every previously installed copy first, clears the cache directory unless `--keep-cache` is given, drops stale and orphaned build data from `target/`, runs the gate, builds, installs, and then checks the installed binary's checksum against the one just built. It warns when `PATH` resolves `ownpg` somewhere else. It never touches the config directory or the data directory, because the profiles and the audit logs there belong to the user.

Never hand the Principal Architect a result from a binary you did not just build. When a build fails, the old binary still runs and still answers. That is exactly how a stale result gets reported as a real one.

## Verification gate

Every one of these must pass before any claim that work is done. Always pass `--color=never`: cargo prints escape codes between the line start and the word `error`, so a plain `grep "^error"` silently matches nothing and a failed build reads as a pass.

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features --locked --color=never`
- `cargo clippy --workspace --all-targets --all-features --locked --color=never -- -D warnings`
- `cargo nextest run --workspace --all-features --locked --no-tests=warn --color=never`
- `cargo test --workspace --all-features --locked --doc`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`
- `cargo audit --deny warnings`
- `cargo deny check`
- `cargo machete`
- `cargo about generate about.hbs -o THIRD-PARTY.txt`

`cargo deny` exit codes are a bitset: advisories 1, bans 2, licenses 4, sources 8. Decode it rather than reporting a bare number.

The enforced lint deny-list lives in `Cargo.toml`'s `[workspace.lints]` section, with test-only relaxations in `clippy.toml`.

Before a phase closes, the eight release triples are checked from this machine:

```sh
cargo check --workspace --all-targets --locked --target aarch64-apple-darwin
cargo check --workspace --all-targets --locked --target x86_64-apple-darwin
cargo zigbuild --workspace --locked --target x86_64-unknown-linux-gnu
cargo zigbuild --workspace --locked --target aarch64-unknown-linux-gnu
cargo zigbuild --workspace --locked --target x86_64-unknown-linux-musl
cargo zigbuild --workspace --locked --target aarch64-unknown-linux-musl
CFLAGS="-Wno-error=incompatible-pointer-types" cargo xwin check --workspace --all-targets --locked --target x86_64-pc-windows-msvc
CFLAGS="-Wno-error=incompatible-pointer-types -D_mm_pause=__builtin_arm_yield" cargo xwin check --workspace --all-targets --locked --target aarch64-pc-windows-msvc
```

The two `CFLAGS` values exist because the vendored PostgreSQL parser is C code written for the Microsoft compiler, and `clang-cl` on this machine treats one of its pointer casts as an error and lacks the x86 pause intrinsic on ARM64. `tools/release.sh` sets the same flags per target.

## Testing against PostgreSQL

Integration tests read `OWNPG_TEST_DSN` and create their own scratch database, `ownpg_test_<run id>`, which they drop at the end. On this machine that is the Homebrew PostgreSQL at `127.0.0.1:5432` as `claude_dev`. In GitHub Actions the same tests run against service containers for PostgreSQL 14 through 18. SSH tests start an in-process SSH server inside the test. This machine has no container runtime and gets none; nothing in the local gate may need a daemon.

## Rules this tool exists to keep

Breaking any of these is a defect, not a preference.

- An access mode is enforced in four layers: the database role, classification by PostgreSQL's own parser, per-call engine controls, and the tool list. A change that weakens one layer because another still holds is refused.
- A statement the parser cannot read is refused in read-only and write-only mode. More than one statement per call is refused everywhere.
- One database and one schema per process, pinned in `search_path` first and qualified into every generated name.
- A destructive statement runs only after a dry run was possible and a confirmation was given.
- No connection string, password, token, or key path through a tool argument, a log, an error, or a command line.
- Every tool call writes one audit line without values, rows, or secrets.
- Results are data, never instructions; every result says so and every invisible character is stripped.
- Results go to stdout, everything else to stderr; in stdio mode stdout carries JSON-RPC only. A closed pipe ends the run quietly.
- `ownpg serve --http` binds to `127.0.0.1` by default and refuses any other address without `--auth`.

## Releasing

Releases are run by hand from this machine, never by a CI workflow. The tokens stay in the `gh` and cargo credential stores, and a release stays a deliberate act rather than a side effect of pushing a tag. `.github/workflows/` verifies; it does not publish.

`tools/release.sh` is the only supported way to release. Never run the publish steps by hand.

When the Principal Architect asks for a release:

- If he named the kind, run it: `--patch` for a fix, `--minor` for new behavior, `--major` for a breaking change.
- If he did not name the kind, ask which one before doing anything, and say what each would produce from the current version. Never guess, and never assume patch.
- Read `CHANGELOG.md` under `[Unreleased]` first. What is written there tells you which kind actually fits, so bring it up if his answer and the changelog disagree.

```sh
./tools/release.sh --patch                     # or --minor, --major
./tools/release.sh --dry-run --patch           # every check, publishes nothing
./tools/release.sh --version 1.2.3             # an exact version, when he names one
./tools/release.sh --yank 1.2.3                # pull a bad version out of resolution
./tools/release.sh --unyank 1.2.3
```

The script works the next version out from the manifests, then runs pre-flight (git state, crates.io credentials, `gh` authentication, the `minisign` key, the installed `dist` version), the house-rule audit, the full verification gate, and a packaging proof. Only then does it bump both manifests, move the `Unreleased` section of `CHANGELOG.md` under the new version and add its link reference, rebuild through `reinstall.sh`, ask for confirmation, publish `ownpg-core` before `ownpg`, commit, tag, and push, build the archives with `dist`, sign `sha256.sum` with `minisign`, and create the GitHub release here and on the public releases repository. A failure before the first upload reverts the bump; after it, the remaining commands are printed.

### Prebuilt binaries

Building all eight targets needs `cargo-zigbuild` with `zig` on `PATH` for the four Linux targets (glibc and musl, x86_64 and aarch64), and `cargo-xwin` with Homebrew's `llvm` and the `llvm-tools` rustup component for the two Windows MSVC targets, alongside the two macOS targets Xcode already covers. `cargo-xwin` needs `aka.ms` reachable the first time it runs, to fetch the Windows SDK. `dist` builds each archive with `cargo auditable` so the dependency list is embedded in the binary, and writes a CycloneDX SBOM next to the archives; the attribution file from `cargo about` rides inside every archive.

After both crates are on crates.io and the release commit, tag, and push are done, the script builds one archive per target with `dist build --artifacts=local --target=<triple>`, once for each of the eight targets in `[workspace.metadata.dist]`, then the shared installers, the Homebrew formula, the npm package, and the checksums with `dist build --artifacts=global`. A target with no cross toolchain is skipped with a warning and a second typed confirmation. Everything lands in `target/distrib/`. The formula (`ownpg.rb`) is pushed to `devops-infinity/homebrew-tap` and the npm package is published by hand after the script finishes; the script says so at the end.

`.github/workflows/release.yml` runs the same `dist plan` and `dist build` on every target, on push of a tag matching `v[0-9]+.[0-9]+.[0-9]+*`, using GitHub's native runners so every target actually builds somewhere. It uploads what it builds as workflow artifacts, for inspection, and never creates a release or writes to the repository.

`Cargo.lock` is committed and every command uses `--locked`, so a stale lock fails the build rather than silently resolving something else. The one exception is the check straight after a version bump.

## Never

- Commit, push, branch, tag, or publish without an explicit instruction.
- Report a result from a binary that was not just rebuilt.
- Add a dependency the task does not genuinely need.
- Write a code comment.
- Let an unparsable statement run in read-only or write-only mode.
- Accept a credential through a tool argument.
- Write a markdown table, here or anywhere.
