# ADR-0020: Private source repository, public releases repository, and hand-run releases

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

`reserve` keeps its source in one repository and publishes its artifacts and takes its bug reports in a second, public `reserve-releases` repository; its `Cargo.toml` names that public repository as the crate's `repository` field, and its releases are built by hand on the Principal Architect's machine through `tools/release.sh` rather than by a GitHub Actions job. OwnPG will live under the `devops-infinity` organization, will be published on crates.io, in a Homebrew tap, on npm, as MCPB bundles, and in the MCP Registry, and has one maintainer who takes no outside code contributions. Where does the source live, who can see it, where do users report problems, and what builds a release?

## Decision Drivers

- The source stays private; users need the binary, the formula, the bundle, and a place to report bugs, not the source.
- The release must be reproducible from a written script that one person runs, with no credential in the repository.
- Every public channel must point at one public place for issues and artifacts.
- The blueprint's supply-chain and provenance items must still be met without CI-built artifacts.
- The single-owner risk must be written down where a user can read it.

## Options Considered

### Option A: Private `devops-infinity/ownpg`, public `devops-infinity/ownpg-releases`, hand-run releases with a stored token

The source, the CI workflows, the planning pack, and the ADRs live in the private repository. The public repository holds the GitHub Releases (archives, checksums, the `minisign` signature, the SBOM, the attribution file, the MCPB bundles), the README that every channel links to, the issue templates (bug report, connection problem, and a pointer to the private security channel), and the `minisign` public key; `cargo binstall`, the shell and PowerShell installers, the Homebrew formula, and the npm installer all resolve to it, and `Cargo.toml` names it as `repository` and `homepage` so crates.io and the registry link there. Outside code contributions are not accepted; the public repository's README says so, and the issue templates ask for reports, not patches. Releases are built by hand on the Principal Architect's machine: `tools/release.sh` runs the gates, publishes `ownpg-core` then `ownpg` to crates.io, runs `dist build` for the eight targets through the cross toolchains, signs `sha256.sum` with `minisign`, creates the GitHub release on the private repository and again on the public repository, and pushes the tap formula and the npm package; it authenticates with the `gh` CLI's stored token and the crates.io token from the cargo credential store, never with a token in the repository, a session transcript, or a script. GitHub Actions runs the checks, the PostgreSQL container matrix, and the remote-mode image build, and never publishes a release. The README of both repositories states that OwnPG has one maintainer and one publisher, that bug reports go to the public repository, and that a successor will be named before the project is relied on by others.

- Pros: the exemplar already proves this shape; `tools/release.sh` and the `repository` field transfer with a rename.
- Pros: private source, public artifacts, one issue tracker, no CI secrets.
- Cons: the release depends on one machine and one person; provenance rests on the `minisign` signature rather than a GitHub attestation.
- Cons: the public repository holds no source, so a user who wants to read the code reads the crates.io tarball instead.

### Option B: One public repository with CI-built releases

- Pros: attestations, contributors, and one place for everything.
- Cons: the source becomes public, which the Principal Architect does not want; the release needs repository secrets; outside contributions arrive whether wanted or not.

### Option C: Private repository only, crates.io as the sole channel

- Pros: nothing public to maintain.
- Cons: no issue tracker for users, no installers, no Homebrew or npm resolution target, no MCPB host, and the registry entry needs a public repository URL.

### Option D: Do nothing

- Cons: the repository would be created with defaults and the release model would be improvised at 0.1.0.

## Decision

We will keep OwnPG's source in the private `devops-infinity/ownpg` repository, publish every artifact and take every bug report in the public `devops-infinity/ownpg-releases` repository, build every release by hand on the Principal Architect's machine through `tools/release.sh` with tokens held only in the `gh` and cargo credential stores, accept no outside code contributions, and record the single-owner risk in both READMEs.

Option A won because it is the proven `reserve` shape, it meets the visibility requirement, and it gives users one public place for artifacts and reports without exposing the source or storing a secret anywhere a session can read it.

## Consequences

- Positive: `cargo install`, `cargo binstall`, `brew`, `npm`, the MCPB bundle, and the registry all resolve to the public repository, and every README links to the same issue tracker.
- Positive: no release credential exists in any repository, workflow, or transcript.
- Negative: a release cannot happen while the Principal Architect or his machine is unavailable; the hotfix and security paths in ADR-0019 inherit that limit.
- Negative: provenance is a signature and a checksum, not a build attestation; the README explains how to verify it.
- Neutral: the planning pack and the ADRs stay in the private repository; the public README carries the user-facing documentation only.

## Reversibility

Cheap to reverse: making the source repository public, moving the issue tracker, or adding a CI release job are settings and a workflow, not code changes. Revisit when a second maintainer joins or when the source is opened.

## Sources

- `/Users/sharkar/Git-Repositories/reserve/Cargo.toml` (`repository` naming `devops-infinity/reserve-releases`, read 2026-09-12)
- `/Users/sharkar/Git-Repositories/reserve/tools/release.sh` and `research/07-reserve-reuse-map.md`, MAP-6 and GAP-1 through GAP-4
- `0017-distribution-channels-and-release-pipeline.md` and `0019-release-versioning-support-deprecation-and-retirement-policy.md`
- User decisions on 2026-09-12: "Private source, public releases repo"; "Keep hand-run builds on your machine, as reserve does"; "Single owner for now, risk recorded"
