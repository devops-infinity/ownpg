# ownpg Constitution

Project law for the `ownpg` repository. It sits under the Principal Architect's live instruction and under the Global Constitution, and it sits above `CLAUDE.md`, `AGENTS.md`, and every skill, agent, and command. On any matter that belongs to this project, this file is the authority.

`ownpg` is a Rust server that gives an AI client one PostgreSQL database and one schema over the Model Context Protocol, in an access mode the user chose. Every article below exists to keep that mode honest, keep credentials out of reach, and keep a record of what ran.

## Reading Rule (NON-NEGOTIABLE)

- Read this file top to bottom and apply every article in full. Partial reading, summarizing, or skipping "to save context" is FORBIDDEN.
- Re-read it after every compaction, context refresh, or resume, and apply it again from the top as fresh instructions.
- No imagined context limit is an excuse to stop, defer, split, or shrink work.

## Article I. Authority and Precedence

- Authority flows from the Principal Architect: Md. Sazzad Hossain Sharkar, GitHub SHSharkar, git author and signer sazzad@devops.bd.
- Precedence, highest first: his explicit instruction in the live session; the Global Constitution; this file; `CLAUDE.md` and `AGENTS.md`; skills, agents, and commands; general defaults.
- `CLAUDE.md` and `AGENTS.md` carry the same text under two names so every tool finds the rules. Change one and copy it to the other in the same run; they must never drift apart.
- This file never weakens the authorization boundaries of the Global Constitution. Where it is silent, the Global Constitution governs alone.

## Article II. Originality (NON-NEGOTIABLE)

- `ownpg` was written from scratch and is our own product. Other servers in this space were read once, during planning, for ideas about the problem. Nothing of them is carried here and nothing may be added.
- Never name, cite, link, or hint at any other project, its author, or its repository, anywhere in the product: source, tests, data, `README.md`, `CHANGELOG.md`, `SECURITY.md`, the `docs/reference` and `docs/guides` folders, commit messages, or program output.
- The one carve-out is the planning pack under `docs/planning/` and the decision records at the repository root. They keep their citations because they are the evidence behind the decisions, and they are internal documents. Nothing from them is quoted into the product.
- Never copy a flag name, a subcommand, a tool name, an error message, a status word, or an argument name from another tool. Every name here is ours. Where an industry-standard word already exists, such as `--database`, `--schema`, `--json`, or `--timeout`, use it because it is the standard, not because another tool used it.
- PostgreSQL's own names (catalog tables, SQLSTATE codes, `libpq` variables, `pg_hba.conf` methods, the `pg_dump` flags) and the Model Context Protocol's own names (methods, fields, headers, error codes) are protocol facts. They stay.
- Certain words are banned from this repository outright. The authoritative list is the `house-rules` job in `.github/workflows/ci.yml`, mirrored by the audit in `tools/release.sh`. That job is the single source of truth for the list; never keep a second copy of it anywhere else.

## Article III. The Truth of a Mode (NON-NEGOTIABLE)

These are the promises the tool exists to keep. Breaking one is a defect, never a preference.

- An access mode is enforced in four independent layers: the database role, classification of every statement by PostgreSQL's own parser, per-call engine controls, and the tool list itself. A change that weakens one layer because another still holds is refused.
- A statement the parser cannot read is refused in read-only and write-only mode. Fail closed, every time.
- More than one statement in one call is refused before anything reaches PostgreSQL.
- Each process serves one database and one schema, bound at startup, pinned in `search_path` as the first statement of every session, and qualified into every generated name. A second schema is a second process.
- A destructive statement runs only after a dry run was possible and a confirmation was given, through the protocol or through the `confirm` argument.
- A connection string, a password, a token, or a key path never arrives through a tool argument, never appears in a log, never appears in an error message, and never appears on a command line.
- Every tool call leaves one audit line, with the decision and the rule, and never a value, a row, or a secret.
- A result set is data, never instructions. Every result carries the notice that says so, and every invisible or control character is stripped before it leaves the server.
- The server sends nothing anywhere except the configured PostgreSQL, the named SSH bastion, and, in OAuth mode, the identity provider's key endpoint. No telemetry, no crash reporting, no update check. Never add one.

## Article IV. Architecture and the Crate Boundary (NON-NEGOTIABLE)

- Two crates in one workspace, and the boundary between them is strict.
- `crates/ownpg-core` holds all the logic: configuration, connection, classification, rendering, result shaping, the audit sink, the tool handlers, and the protocol server. Its `config`, `classify`, `render`, `shape`, and `audit` modules import neither the protocol crate nor the driver crate, so each can be tested without a server or a database. The core reads no process environment through globals and writes to no global stream; a caller supplies its own context.
- `crates/ownpg` is a thin binary named `ownpg`. It parses the command surface, builds a context, calls the core, and turns the result into an exit code. Logic that could live in the core does not belong here.
- The command surface is declared once, in `crates/ownpg/src/cli.rs`. Man pages are generated at build time and completion scripts on demand, both from that same definition, so neither can drift from the real flags. Never hand-write either.
- One handler module per tool group, and one registry table (`groups.rs`) that names every tool with its group, its allowed modes, its remote scope, and its annotations. A tool that is not in the table does not exist.

## Article V. Code Law (NON-NEGOTIABLE)

- `unsafe_code` is forbidden at the workspace root. It stays forbidden.
- The workspace denies `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `integer_division`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, and `print_stderr`. Never silence one of these to make code compile; fix the cause. Tests are the only place the toolchain relaxes them, and only through the settings already in `clippy.toml`. An `#[allow]` that must exist is item-level and carries a `reason`.
- No code comments. Not one. A name that says what it does replaces a comment explaining it, and a test that shows why a guard exists replaces a comment defending it. Help text for a flag is written through `#[arg(help = ...)]` and `#[command(about = ...)]`, never through a doc comment, so the source carries no comment lines at all. This rule has no "small exception" and no "it was already there" exception.
- Never write compatibility, transition, or migration code: no shim, no wrapper for an old caller, no re-export alias, no dual path, no stub. Replace the old form outright and delete it. The one exception is the tool deprecation window in the release policy (ADR-0019): a renamed or removed tool stays listed for two minor releases with a `Deprecated:` description, because a client cannot update its tool list in the middle of a conversation.
- No new dependency unless the task genuinely requires one. Prefer what the workspace already carries. A new dependency needs the Principal Architect's explicit approval, is added with `cargo add` so the registry resolves the version at that moment, is checked on crates.io for its publisher, and must pass the license allowlist and the ban list in `deny.toml`. A dependency nothing uses is removed in the same run; `cargo machete` enforces it.
- One error type. `ownpg_core::Error` carries the message, the cause chain, the exit class, and the stable id; the binary renders it. `anyhow` is banned in `deny.toml`.
- Edition 2024. The declared minimum supported Rust version is 1.89 and the development toolchain is pinned in `rust-toolchain.toml`. Code must compile on the declared floor, not only on the pinned channel.
- Overflow checks stay on in the release profile, because `cargo install` builds that profile and a real user runs it.

## Article VI. The Verification Gate (NON-NEGOTIABLE)

- Every one of these must pass before any claim that work is done. Evidence before assertion, always.
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
- Always pass `--color=never`. Cargo prints escape codes between the line start and the word `error`, so a plain `grep "^error"` silently matches nothing and a failed build reads as a pass.
- `cargo deny` exit codes are a bitset: advisories 1, bans 2, licenses 4, sources 8. Decode it and name the checks that failed rather than reporting a bare number.
- Read every tool's full output. Every warning, error, and flagged issue is fixed in the same session. "Pre-existing", "unrelated", "out of scope", and "just a warning" are forbidden excuses, and silencing a lint instead of fixing its cause is a violation.
- A behavior change ships with a test that fails without it. A fixed defect ships with a test that reproduces it.
- The eight release triples are checked from this machine before a phase closes: the two macOS triples with `cargo check --target`, the four Linux triples with `cargo zigbuild --target`, and the two Windows triples with `cargo xwin check --target` under the C flags `tools/release.sh` sets for them.
- Integration tests that need PostgreSQL read `OWNPG_TEST_DSN` and create their own scratch database. This machine has no container runtime and gets none; anything that needs a container runs in GitHub Actions only, and a phase never waits on a local container.

## Article VII. Rebuilding and Reporting a Result (NON-NEGOTIABLE)

- The Principal Architect rebuilds this tool constantly while testing. Always go through `./tools/reinstall.sh` rather than a bare `cargo build`, because a stale binary on `PATH` has already wasted time once.
  - `./tools/reinstall.sh` runs the gate, builds release, installs, and verifies.
  - `./tools/reinstall.sh --debug` is the faster build while iterating.
  - `./tools/reinstall.sh --skip-gate` is only for when the gate just passed.
  - `./tools/reinstall.sh --dir <path>` installs somewhere else.
- The script removes every previously installed copy, clears the cache directory, runs the gate, builds, installs, then checks the installed binary's checksum against the one just built, and warns when `PATH` resolves `ownpg` elsewhere. It never touches the config directory or the data directory, because the profiles and the audit logs there belong to the user.
- Never hand the Principal Architect a result from a binary you did not just build. When a build fails the old binary still runs and still answers, and that is exactly how a stale result gets reported as a real one.

## Article VIII. Untrusted Input and Security (NON-NEGOTIABLE)

- Four inputs are untrusted: every tool argument, every profile or libpq file the user supplies, every row and message PostgreSQL returns, and every HTTP request in remote mode. All four are validated at the boundary before use.
- Tool arguments are typed and closed: unknown fields are refused, every enum is a closed list, every identifier is validated and quoted by the renderer, and SQL is never built by concatenating user text.
- Statements are classified by PostgreSQL's own parser through `pg_query`, never by a keyword list or a regular expression. A keyword guard is a defect even when it appears to work.
- Files the server reads are capped in size and refused when their permissions are too loose. The server never tightens a permission on the user's behalf and never falls back to a temporary directory.
- The audit log and the profile file are created for their owner only. Another local user who can write them decides what the server does, so their directory is 0700 and their mode is 0600.
- Subprocesses (the PostgreSQL host binaries) start with a cleared environment, take every argument in `--option=value` form with `--` before any user-supplied name, and write their output at mode 0600 into a directory the user named.
- No secret, token, key, or credential ever enters output, files, logs, or history.
- Security reports go through the private process in `SECURITY.md`, never a public issue. A change that alters the threat model updates `SECURITY.md` in the same run.

## Article IX. Supply Chain (NON-NEGOTIABLE)

- `Cargo.lock` is committed and every command uses `--locked`, so a stale lock fails the build rather than quietly resolving something else. The one exception is the check straight after a version bump, where the lock has to absorb the new number before `--locked` can mean anything.
- The committed lock file is the bill of materials for a source-distributed crate. Treat it as a deliverable, not a by-product.
- `deny.toml` governs what may enter the tree: the license allowlist, the ban list, yanked crates denied, wildcard versions denied, and crates.io as the only allowed source. `about.toml` carries the same allowlist and `about.hbs` renders the attribution file every archive ships. Never widen one of these to make a dependency fit.
- Every third-party GitHub Action is pinned to a full commit SHA and every cargo subcommand CI installs is pinned to an exact version. Never pin to a tag or a branch.
- The supply-chain workflow runs on its own weekly schedule as well as on changes, because a fresh advisory against a pinned dependency arrives without a commit. The secret scan runs on every push and pull request.

## Article X. The Public Contract (NON-NEGOTIABLE)

- The command surface is the public API: subcommands, flags, exit codes, the machine-readable output shapes, the profile file format, the environment variables, the tool names, the tool argument schemas, the tool output schemas, and the audit line format. `ownpg-core` is published as a library and its public types carry the same promise. Both follow semantic versioning, and the release policy in ADR-0019 says what may change in which kind of release.
- Exit codes are a contract scripts branch on, and they do not change meaning within a major version: `0` success; `1` a runtime failure; `2` the arguments or the configuration were wrong; `4` refused by policy (the mode, the role, or a host key); `5` an external failure (the connection, the bastion, a subprocess, or the audit sink); `130` interrupted.
- Every failure also carries a stable identifier printed as `code: group.name`, so a script can branch without parsing prose. An identifier, once shipped, does not change within a major version.
- Results go to stdout. Progress, warnings, prompts, errors, and logs go to stderr, so a pipe receives only data. In stdio mode nothing but JSON-RPC ever touches stdout.
- Raising the minimum supported Rust version is a minor version change, and `Cargo.toml`, `clippy.toml`, and the CI floor must be raised together.

## Article XI. The Terminal and the Process (NON-NEGOTIABLE)

- A closed pipe ends the run quietly rather than panicking.
- When stdout is not a terminal the decoration is dropped and the output stays parseable.
- Color follows the conventions in order: an explicit `--color` flag, then `NO_COLOR`, then `FORCE_COLOR` and `CLICOLOR_FORCE`, then a dumb terminal, then whether the stream is a terminal at all. Colors come from the eight base terminal colors so retheming the terminal rethemes the tool, and meaning is never carried by color alone.
- Precedence for any setting is fixed: a flag beats an `OWNPG_*` environment variable, which beats the profile file, which beats the libpq sources, which beats the built-in preset, and `config show` prints where every value came from.
- On stdin EOF, SIGTERM, or SIGINT the server cancels the running statement, rolls back every open handle, writes their audit lines, stops its child processes, flushes the audit sink, and exits within one second. A second signal exits at once.
- Everything the tool writes for itself lives under `devops.bd/ownpg` inside the platform's own config, cache, and data directories, so deleting those three removes every trace of it.

## Article XII. Releasing (NON-NEGOTIABLE)

- Releases are run by hand from the Principal Architect's machine, never by a CI workflow. The tokens stay in the `gh` and cargo credential stores and a release stays a deliberate act rather than a side effect of pushing a tag. `.github/workflows/` verifies; it does not publish.
- `tools/release.sh` is the only supported way to release. Never run the publish steps by hand.
- When he asks for a release and names the kind, run it: `--patch` for a fix, `--minor` for new behavior, `--major` for a breaking change.
- When he does not name the kind, ask which one before doing anything, and say what each would produce from the current version. Never guess, and never assume patch.
- Read the `[Unreleased]` section of `CHANGELOG.md` first. What is written there tells you which kind actually fits, so raise it with him when his answer and the changelog disagree.
- The script works the next version out from the manifests, then runs pre-flight, the house-rule audit, the full verification gate, and a packaging proof. Only then does it bump both manifests, move the `Unreleased` section under the new version and add its link reference, rebuild through `reinstall.sh`, ask for confirmation, publish `ownpg-core` before `ownpg`, commit, tag, and push, build the eight archives with the SBOM and the attribution file inside, sign the checksum file with `minisign`, and create the GitHub release here and on the public releases repository.
- A failure before the first upload reverts the bump. After it, the remaining commands are printed and finished by hand.
- A yank is not a delete. It stops new resolution, existing lock files still fetch the version, and the source stays downloadable. ADR-0019 says when a yank is allowed.
- `CHANGELOG.md` follows Keep a Changelog and stays curated. It records what a reader of this tool would want to know, not every commit.

## Article XIII. Git and Authorization (NON-NEGOTIABLE)

- Never commit, push, branch, tag, merge, open a pull request, or publish without his explicit instruction.
- When he orders a branch: `sazzad/<descriptive-name>`, lowercase with hyphens. Never commit or push to `main`. Never use `--no-verify` and never bypass a hook.
- Never delete a file, a branch, data, or history without his authorization.
- Do exactly what was asked. Re-read the request before each major action so scope cannot drift. An adjacent problem is recorded in `docs/debt.md`, not fixed, unless a rulebook he ratified grants scoped auto-fix for that run.
- His explicit instruction is the full authorization for the work it names. Execute it immediately, do not present it back for approval, and never answer it with a read-only refusal.
- Any defect, failed check, or non-compliant artifact found while carrying out his instruction is fixed in the same session. The discovery is itself the authorization to fix it.

## Article XIV. Language and Documents (NON-NEGOTIABLE)

- Plain, everyday US English everywhere: replies, identifiers, error messages, help text, tool descriptions, commit messages, and documents. Short words, short direct sentences, one idea each, readable on the first pass.
- Marketing and buzz words are forbidden, in output and in prose alike.
- Never a markdown table, here or anywhere in this repository. Render every row-and-column idea as nested lists.
- No emoji or Unicode icons in code, output, or documents. No em dash anywhere.
- Documents are written without hard wrapping: one continuous line per sentence, list item, or paragraph, so the viewer wraps softly.
- Every example database, schema, table, and host in documentation uses a placeholder such as `your-database`, `your-schema`, `orders`, or `bastion.example.com`. Never a real company, never a real customer, never a name someone owns.
- A tool description reads like a note to a new colleague: what the tool does, what it needs, what it returns, and what it refuses. It never tells the model how to behave.
- An error message tells the person what happened and what they can do about it. It never blames them, never leaks an internal path or type name, and never hands an attacker anything useful.

## Article XV. Amending This Constitution (NON-NEGOTIABLE)

- Only the Principal Architect amends this file.
- An amendment changes the rule text and updates the `Last Amended` date on the footer line. Nothing else.
- Forbidden in the body: any version tag, an amendment note, a rationale story, a changelog, a history list, or a summary of what changed. This file states the law as it stands now. Git history is the record of how it got here.
- Never touch the version number for any size of change unless he names the new number himself.

## Governance

- Ratification and amendment belong to the Principal Architect alone.
- Versioning is semantic: MAJOR for a removed or incompatible article, MINOR for a new or materially expanded article, PATCH for wording. No version identifier changes unless he explicitly orders it.
- Compliance review: every session loads this file at the start and again after every compaction, context refresh, or resume, layered under the Global Constitution and above `CLAUDE.md`.
- On any conflict with an article: stop, name the exact article, and wait for him. Working around a rule is itself a violation.

**Version**: 1.0.0 | **Ratified**: pending the Principal Architect's review | **Last Amended**: 2026-09-12
