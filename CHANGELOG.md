# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The `ownpg` command surface with `man` and `completions`, manual pages generated at build time, completion scripts for bash, elvish, fish, powershell, and zsh, and a `--version` line that carries the short commit and the build date.
- The `ownpg-core` error type with six exit classes (`0`, `1`, `2`, `4`, `5`, `130`) and a stable `group.name` identifier on every failure.
- Statement counting through the PostgreSQL parser, the first piece of the classifier.

[Unreleased]: https://github.com/devops-infinity/ownpg-releases/releases
