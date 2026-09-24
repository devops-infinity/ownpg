# Releasing OwnPG

This is the checklist for shipping an OwnPG release from the maintainer's Mac. One release puts the same version on crates.io, on the GitHub release pages of `devops-infinity/ownpg` and `devops-infinity/ownpg-releases`, in the Homebrew tap, on npm, and in the MCP Registry. `tools/release.sh` does all of it except the MCP Registry, which you publish by hand at the end.

A release takes about an hour. Most of that is building the eight prebuilt binaries.

## One-time setup

Do this once per machine. `tools/release.sh` checks these before it publishes anything, and it stops with the fix when one is missing.

### Tools

- Rust: rustup installs the toolchain that `rust-toolchain.toml` names. Add the seven cross targets, plus the LLVM tools the Windows ARM64 build needs:

  ```bash
  rustup target add x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-msvc aarch64-pc-windows-msvc
  rustup component add llvm-tools
  ```

- Homebrew packages:

  ```bash
  brew install zig jq minisign gh mcp-publisher gitleaks shellcheck shfmt bats-core
  ```

- Cargo tools. `dist` has to match `cargo-dist-version` in `Cargo.toml` exactly, which is 0.32.0 today:

  ```bash
  cargo install --locked cargo-nextest cargo-audit cargo-deny cargo-machete cargo-about cargo-auditable cargo-cyclonedx cargo-semver-checks cargo-zigbuild cargo-xwin
  cargo install --locked cargo-dist --version 0.32.0
  ```

- The MCPB bundle tool: `npm install -g @anthropic-ai/mcpb`
- Git hooks, once per checkout: `tools/install-hooks.sh`. The pre-commit hook scans for secrets with gitleaks, and the pre-push hook runs `tools/verify.sh`.

### Accounts and keys

- crates.io: run `cargo login` with a token from an account that owns both `ownpg-core` and `ownpg`. `CARGO_REGISTRY_TOKEN` in the environment works too.
- GitHub: run `gh auth login` with push access to `devops-infinity/ownpg`, `devops-infinity/ownpg-releases`, and `devops-infinity/homebrew-tap`.
- Signed tags: when `user.signingkey` is set in your git config, the release tag is signed. Without it, the tag is annotated but unsigned.
- minisign: the release signs `sha256.sum` with the secret key at `~/.minisign/minisign.key`. The key has no password, so the release runs unattended. Its public key is `minisign.pub` at the repository root (key ID `E60F7CB18005C98F`), and the README prints it for users. Keep a copy of the secret key in your password manager. If it's ever lost, make a new pair with `minisign -G -W -p ~/.minisign/minisign.pub -s ~/.minisign/minisign.key`, then commit the new `minisign.pub` and update the key in the README.
- npm: `~/.npmrc` holds a granular access token for `@devops-infinity/ownpg`, with read and write access and "Bypass two-factor authentication" checked. `npm whoami` should print `sharkar`. Running `npm login` replaces that token, and the next release then stops at the npm check, so put a new bypass token back in `~/.npmrc` after any `npm login`. Renew it before it expires.
- Test database: the gate runs live tests against `OWNPG_TEST_DSN`, a libpq connection string for a role that can create databases. On this Mac, that's `host=127.0.0.1 port=5432 user=claude_dev dbname=postgres`. Export it before you push or release.

### Optional: signed binaries

Without these settings, the macOS and Windows binaries ship unsigned. Installs through Homebrew, npm, cargo, or the shell installer don't mark the file as downloaded, so the OS warnings mostly hit people who download an archive in a web browser.

- macOS: set `OWNPG_CODESIGN_IDENTITY` to a Developer ID Application identity, and `OWNPG_NOTARY_PROFILE` to a profile saved with `xcrun notarytool store-credentials`. This needs a paid Apple Developer Program membership.
- Windows: the script signs with `osslsigncode` and a `.p12` file (`OWNPG_WINDOWS_SIGN_CERT` and `OWNPG_WINDOWS_SIGN_PASS`). Certificates sold since June 2023 keep the key on hardware and can't be exported to a `.p12`, so this step has to change before a Windows certificate can be used.

## Cutting a release

### 1. Prepare the version on a branch

The release script never commits to `main`, so the version bump reaches it through a pull request first:

```bash
git switch main && git pull
git switch -c sazzad/prepare-X-Y-Z
tools/release.sh --prepare --patch
```

Use `--minor`, `--major`, or `--version X.Y.Z` in place of `--patch` as needed. `--prepare` sets the new version in `Cargo.toml`, `crates/ownpg/Cargo.toml`, `Cargo.lock`, `mcpb/manifest.json`, and `server.json`. It adds an empty `## [X.Y.Z]` heading to `CHANGELOG.md` and moves the changelog links. Write the entries under that heading, because the release refuses an empty section. Then commit, push, open a pull request, and squash-merge it.

### 2. Run the release from main

```bash
git switch main && git pull
export OWNPG_TEST_DSN='host=127.0.0.1 port=5432 user=claude_dev dbname=postgres'
tools/release.sh --version X.Y.Z
```

The script asks you to type the version before it publishes. Add `--yes` to skip that. Then it runs, in order:

1. Pre-flight: the tools, a clean tree level with `origin/main`, the crates.io, GitHub, and npm logins, the minisign key, the `dist` version, and the LLVM tools.
2. `tools/verify.sh`, then `cargo semver-checks` against the published `ownpg-core`.
3. A packaging check, then a release build installed to `~/.local/bin`.
4. Publish `ownpg-core`, wait until crates.io serves it, then publish `ownpg`.
5. Tag `vX.Y.Z` and push the tag. `main` itself isn't pushed.
6. Build the eight binaries with dist, run the macOS one as a check, pack six MCPB bundles, and sign `sha256.sum`.
7. Write `target/distrib/server.json` for the MCP Registry.
8. Create the GitHub release on both repositories, push the Homebrew formula, and publish the npm package.

It ends with `released X.Y.Z`.

### 3. Publish to the MCP Registry

Do this by hand after the release. Since registry 1.8.0, the browser login (`mcp-publisher login github` on its own) grants only your personal namespace, so it can't publish `io.github.devops-infinity/ownpg` ([registry issue #1468](https://github.com/modelcontextprotocol/registry/issues/1468)). Log in with a token instead:

1. Create a classic token with only `read:org` and a short expiry: https://github.com/settings/tokens/new?scopes=read:org&description=mcp-registry-publish
2. Copy it, then run:

   ```bash
   cd target/distrib
   mcp-publisher login github --token "$(pbpaste)"
   mcp-publisher publish
   ```

3. Delete the token on the same GitHub page.

The registry lists the crates.io install only because the crate README shows `mcp-name: io.github.devops-infinity/ownpg`. The Status section of `README.md` carries that line, so keep it.

### 4. Check every channel

```bash
curl -s -A 'ownpg-release-check' https://crates.io/api/v1/crates/ownpg | jq -r .crate.max_version
npm view @devops-infinity/ownpg version --color=false
curl -s "https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.devops-infinity/ownpg&version=latest" | jq -r '.servers[].server.version'
curl -sL https://raw.githubusercontent.com/devops-infinity/homebrew-tap/main/Formula/ownpg.rb | grep 'version "'
gh release view --repo devops-infinity/ownpg-releases --json tagName --jq .tagName
```

Each one should print the new version. npm's public side can lag a few minutes behind the publish.

## When something goes wrong

- The release stopped part way: run it again with `--resume`, for example `tools/release.sh --resume --version X.Y.Z`. It skips every step that already happened.
- "the working tree is not clean": commit or stash your changes first.
- "tools/release.sh never commits to main": the version bump isn't merged yet. Go back to step 1.
- A target was skipped for a missing cross toolchain: install it and resume. `OWNPG_ALLOW_PARTIAL=1` publishes the targets that did build, but the installers still offer every target. A target that fails for any other reason stops the release, because that's a real build error.
- "the llvm-tools component is missing": run `rustup component add llvm-tools`.
- "npm is not logged in", or npm asks for a one-time password: the bypass token in `~/.npmrc` is missing, expired, or was replaced by `npm login`. Put a new one there and resume.
- The MCP Registry answers 403 with "You have permission to publish: io.github.SHSharkar/*": you logged in through the browser. Log in again with `--token`, as in step 3.
- The MCP Registry answers 400: fix `target/distrib/server.json`, check it with `mcp-publisher validate`, and publish again.
- A published version is broken: `tools/release.sh --yank X.Y.Z` yanks it from both crates, and `--unyank X.Y.Z` brings it back. Ship the fix as the next version.
- To rehearse without publishing anything: `tools/release.sh --dry-run --patch`.
