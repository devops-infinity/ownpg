# ownpg

PostgreSQL DBA tools for AI clients over the Model Context Protocol, one database and one schema per run.

`ownpg` is one binary. It connects to the PostgreSQL you already have (Homebrew, Postgres.app, Docker, a Linux package, or a server behind SSH) the way `psql` would, with or without a password, and gives an AI client a chosen access mode: read-only, write-only, or read-write. It speaks the current Model Context Protocol revision over stdio first and over Streamable HTTP second.

This is the first build. It carries the command surface, the manual pages, and the shell completions. The server, the connection, and the tools arrive with the next release; `CHANGELOG.md` says what is in each one.

## Contents

- [Requirements](#requirements)
- [Supported platforms](#supported-platforms)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Command reference](#command-reference)
- [Exit codes](#exit-codes)
- [Output and streams](#output-and-streams)
- [Environment](#environment)
- [Architecture](#architecture)
- [Building from source](#building-from-source)
- [Support and compatibility](#support-and-compatibility)
- [Contributing](#contributing)
- [License](#license)

## Requirements

- Nothing at run time beyond the binary. The prebuilt archives are self-contained.
- Rust 1.89 or newer to build from source. The pinned development toolchain is in `rust-toolchain.toml`.

## Supported platforms

Prebuilt binaries are built for eight targets:

- macOS on Apple Silicon (`aarch64-apple-darwin`) and Intel (`x86_64-apple-darwin`)
- Linux with glibc on x86_64 (`x86_64-unknown-linux-gnu`) and aarch64 (`aarch64-unknown-linux-gnu`)
- Linux fully static with musl on x86_64 (`x86_64-unknown-linux-musl`) and aarch64 (`aarch64-unknown-linux-musl`)
- Windows on x86_64 (`x86_64-pc-windows-msvc`) and ARM64 (`aarch64-pc-windows-msvc`)

The minimum operating system versions are recorded here at the first release.

## Installation

Every install path resolves against the public releases repository, `devops-infinity/ownpg-releases`.

- From crates.io, building locally: `cargo install ownpg --locked`
- A prebuilt binary through cargo: `cargo binstall ownpg`
- The shell installer (macOS and Linux) and the PowerShell installer (Windows) are attached to every release.
- Homebrew: `brew install devops-infinity/tap/ownpg`
- npm: `npm install -g @devops-infinity/ownpg`

Every archive carries `sha256.sum`, a `minisign` signature over it (`sha256.sum.minisig`), a CycloneDX SBOM, and `THIRD-PARTY.txt` with every dependency license. The public key for the signature is published in the releases repository.

## Quick start

```sh
ownpg --version
ownpg man
ownpg completions zsh > "${fpath[1]}/_ownpg"
```

## Command reference

- `ownpg man [COMMAND...]`: write the manual page to stdout, for the whole tool or for one command named in full.
- `ownpg completions <SHELL>`: write a completion script to stdout for `bash`, `elvish`, `fish`, `powershell`, or `zsh`.
- `ownpg --version`: the version, the short commit it was built from, and the build date. A build without a repository or without `SOURCE_DATE_EPOCH` prints `unknown` for the part it cannot know.
- `ownpg --help`: the full command surface.

## Exit codes

- `0`: success.
- `1`: a runtime failure inside the tool, such as stdout that could not be written.
- `2`: the arguments or the configuration were wrong.
- `4`: refused by policy: the access mode, the database role, or an unknown host key.
- `5`: an external failure: the connection, the bastion, a subprocess, or the audit sink.
- `130`: interrupted.

Every failure also prints `code: group.name` on stderr, a stable identifier a script can branch on.

## Output and streams

Results go to stdout. Everything else (errors, warnings, logs) goes to stderr, so a pipe receives only data. An error is printed as `error: <what happened>`, then `caused by:` lines for the chain, then `try: <what to do>`, then `code: <identifier>`. A closed pipe ends the run quietly.

## Environment

None in this build. The connection, mode, and profile variables arrive with the server.

## Architecture

Two crates in one workspace:

- `crates/ownpg-core`: the library. The error type with its exit classes and stable identifiers, and the statement classifier built on the PostgreSQL parser.
- `crates/ownpg`: the binary. The command surface, the manual pages generated at build time, the completions, and the error rendering.

The planning pack (research report, PRD, plan, and research notes) is under `docs/planning/`; the decision records are the `NNNN-*.md` files at the repository root.

## Building from source

```sh
git clone <the source repository> ownpg
cd ownpg
./tools/reinstall.sh
```

`tools/reinstall.sh` runs the verification gate, builds the release profile, installs into `~/.local/bin`, and checks the installed binary's checksum against the one just built. The gate is:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features --locked`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo nextest run --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked --doc`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`
- `cargo audit --deny warnings`
- `cargo deny check`
- `cargo machete`

Cross-building the Linux targets needs `cargo-zigbuild` and `zig`; the Windows targets need `cargo-xwin`, LLVM's `clang-cl`, and the `llvm-tools` rustup component. The Windows builds of the vendored PostgreSQL parser need `CFLAGS="-Wno-error=incompatible-pointer-types"` under `clang-cl`, and the ARM64 build also needs `-D_mm_pause=__builtin_arm_yield`; `tools/release.sh` sets both.

## Support and compatibility

`ownpg` has one maintainer and one publisher. Bug reports and questions go to the issues of the public releases repository, `devops-infinity/ownpg-releases`. Security reports follow `SECURITY.md`. Versions follow semantic versioning; before 1.0.0 a minor release may change the command surface, and `CHANGELOG.md` says when it does.

## Contributing

The source repository is private and outside code contributions are not accepted. Bug reports through the releases repository are welcome.

## License

Licensed under either of the MIT license (`LICENSE-MIT`) or the Apache License, Version 2.0 (`LICENSE-APACHE`), at your option.
