# ADR-0019: Release, versioning, support, deprecation, and retirement policy

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The `rust-cli-apps` blueprint (Groups W, X, and Y) and the `building-modules` blueprint (the lifecycle groups) each ask for a written policy before the first release: how versions are numbered, what a user may rely on between versions, how long a PostgreSQL major or a Rust version stays supported, how a tool is retired, how a bad release is undone, how a security fix reaches users, and what happens when the project ends. OwnPG has one maintainer, ships eight targets, publishes two crates, a Homebrew formula, an npm package, MCPB bundles, and a registry entry, and depends on three moving upstreams: the MCP specification, PostgreSQL, and the Rust toolchain. OwnPG's constitution will carry the house rule that no compatibility shim survives a change. What is the policy?

## Decision Drivers

- A user, a script, and an MCP client must know what can change in a minor release and what cannot.
- One maintainer means every path must be short and written down, because there is no one to ask.
- The support window must follow the upstream windows rather than invent longer ones.
- A security fix must reach every install path on the same day.
- Retiring a tool must not break a client mid-conversation, and must not leave a shim behind forever.

## Options Considered

### Option A: Semantic versioning with written windows, a two-minor tool deprecation window, and a same-day security path

Versioning: semantic versioning 2.0.0 for the binary and for `ownpg-core`. The public API for the binary is the command surface (subcommands, flags, exit classes, the `--format json` shapes, the profile file format), the tool contract (tool names, argument schemas, `outputSchema`, `isError` codes), the audit line format, and the environment variables; a change to any of these outside an additive extension is a major release. Before 1.0.0, a minor release may change them with a CHANGELOG entry. `cargo semver-checks` guards `ownpg-core`.

Support windows: PostgreSQL majors follow the PostgreSQL project's window (ADR-0018); a major is dropped in the first minor release after its upstream end of life. The MSRV is a minor-release change, never a patch, and is raised only when a dependency requires it or a toolchain older than twelve months is holding the workspace back. Operating system floors are recorded in the README at the first cross-build and raised only in a minor release. MCP revisions: the primary revision is `2026-07-28`; the `2025-11-25` handshake stays until the client matrix shows no legacy clients in use, and its removal is a major release.

Compatibility formats: the profile file carries `format = 1`; OwnPG reads its own format and the one before it, refuses a newer one with an upgrade message, and never rewrites a profile on read. The audit line carries `v`; a new field is additive, a renamed or removed field bumps `v`, and readers are told to key on `v`. The `--format json` outputs carry `format_version` with the same rule.

Tool deprecation: a tool that is renamed or removed stays listed for two minor releases with a description that starts with `Deprecated:`, names the replacement, and names the removal version; the audit line records calls to it with `rule = "deprecated"`; the CHANGELOG lists it under `Deprecated` on the first release and under `Removed` on the last. This window is the one exception to the constitution's no-shim rule, because a client cannot update its tool list mid-conversation; every other compatibility layer is refused.

Hotfix path: a hotfix branches from the release tag, carries only the fix and its test, bumps the patch version, and ships through the same `tools/release.sh` run as a normal release, to every channel at once. Rollback for a user is a reinstall of the previous version: the installers accept a version, `cargo install ownpg --version <x.y.z> --locked`, `cargo binstall ownpg@<x.y.z>`, `brew install devops-infinity/tap/ownpg@<x.y.z>` when the tap carries the versioned formula, and the previous GitHub release stays downloadable. OwnPG itself never auto-updates.

Yank rule: a crates.io version is yanked only when it is unbuildable or when it carries a security defect with a fixed version already published; a version is never yanked for a feature regret, and a yank is always paired with a CHANGELOG line that names the replacement.

Security fixes: a report arrives through the private channel in `SECURITY.md`; the fix is developed on a private branch, released as a patch on every channel, and followed the same day by a RustSec advisory for `ownpg-core` (a pull request to the advisory database) and a GitHub security advisory on the releases repository; the CHANGELOG entry names the CVE or the advisory id and the affected range. Remediation targets: critical within 7 days of the report, high within 30 days, others in the next minor release.

Retirement: when the project ends, the last release states it in the README and the CHANGELOG, the registry entry is marked deprecated, the crates.io README of the final version carries the notice, the releases repository is archived so its artifacts stay downloadable, and the Homebrew formula is marked deprecated with a reason; nothing is deleted.

- Pros: every question the blueprints ask has one written answer.
- Pros: the deprecation window is bounded, visible in the tool list, and recorded in the audit.
- Cons: the maintainer must run the security path alone; the remediation targets are a promise with one person behind them (ADR-0020 records the risk).

### Option B: Calendar versioning (`2026.9.0`)

- Pros: the version says when it shipped.
- Cons: says nothing about compatibility; `cargo semver-checks` and `cargo binstall` version ranges lose their meaning; the exemplar uses semantic versioning.

### Option C: Semantic versioning with no written windows or paths

- Pros: less to write.
- Cons: every drop, every deprecation, and every hotfix would be decided under pressure, and a user could not plan an upgrade.

### Option D: Do nothing

- Cons: the blueprints' release, compatibility, and lifecycle groups stay open, and the first bad release has no undo path.

## Decision

We will version OwnPG with semantic versioning, follow the upstream support windows, keep profile and audit formats readable one version back, retire tools through a two-minor deprecation window that is the only exception to the no-shim rule, ship hotfixes and security fixes as patches to every channel on the same day with a RustSec advisory, yank only unbuildable or vulnerable versions, and retire the project by archiving rather than deleting.

Option A won because it answers every lifecycle question in advance with rules a single maintainer can follow without judgment calls.

## Consequences

- Positive: a user can read the version and know whether a script or a client configuration still works.
- Positive: the tool list itself announces a retirement, so an agent sees the replacement before the removal.
- Negative: the two-minor window means a renamed tool costs three releases to finish.
- Negative: the same-day security path depends on one person and one release machine (ADR-0020).
- Neutral: before 1.0.0 the command surface may still change in a minor release; the README says so until the 1.0.0 tag.

## Reversibility

Cheap to reverse for the windows and the deprecation length; expensive for the versioning scheme once tags exist. Revisit when a second maintainer joins, when PostgreSQL 14 leaves support on 2026-11-12, and at every MCP revision.

## Sources

- https://semver.org/spec/v2.0.0.html
- https://www.postgresql.org/support/versioning/ (read 2026-09-12)
- https://doc.rust-lang.org/cargo/commands/cargo-yank.html
- https://github.com/rustsec/advisory-db/blob/main/CONTRIBUTING.md
- https://keepachangelog.com/en/1.1.0/
- `~/.claude/skills/rust-cli-apps/references/blueprint-checklist.md`, Groups W, X, and Y
- `~/.claude/skills/building-modules/references/blueprint-checklist.md`, the lifecycle groups
- `0018-postgresql-version-support-and-feature-gates.md`, `0017-distribution-channels-and-release-pipeline.md`, `0020-repository-visibility-and-release-model.md`
- User decisions on 2026-09-12: hand-run releases, eight targets, single owner recorded
