# Security policy

## Reporting a vulnerability

The most recently published version receives security fixes. There is no separate long-term-support branch.

Report privately through GitHub's private vulnerability reporting at [devops-infinity/ownpg-releases](https://github.com/devops-infinity/ownpg-releases/security/advisories/new). Do not open a public issue for a security problem.

Include the affected version, the steps to reproduce it, and the impact.

Expect an acknowledgement within seven days and an assessment within fourteen. A critical issue is fixed within seven days of the report, a high one within thirty, and the rest in the next minor release. A fix ships as a patch release on every install path on the same day, with an advisory on the releases repository and, for `ownpg-core`, a RustSec advisory.

## What this tool does with your data

This build carries the command surface, the manual pages, and the shell completions. It opens no network connection, reads no file the user did not name, and writes nothing beyond its own output. The server, the connection, and the tools arrive with the next release, and this file grows a threat model with them.

There is no telemetry, no crash reporting, and no automatic update check.

## Threat model

The untrusted inputs in this build are the command-line arguments. They are parsed by a typed argument parser, an unknown flag or command is refused with exit code `2`, and a manual page request for a command that does not exist is refused with the stable identifier `command.unknown`.

Building from source runs third-party build scripts, including a C build of the vendored PostgreSQL parser. Those run unsandboxed at build time, as they do for any Rust program. The dependency set is pinned in `Cargo.lock`, license-checked and advisory-scanned by `cargo deny` and `cargo audit` on every change, and the committed lock file is the bill of materials for a source-distributed crate. Every release archive carries a CycloneDX SBOM and the binary embeds its dependency list through `cargo auditable`.
