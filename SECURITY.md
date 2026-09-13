# Security policy

## Supported versions

OwnPG is pre-release software at version 0.1.0, with no version tagged, so only the latest commit on the `main` branch receives security fixes.

## Reporting a vulnerability

This repository is private. Report a vulnerability through GitHub's private vulnerability reporting on the public release repository: https://github.com/devops-infinity/ownpg-releases/security/advisories/new. Do not report a vulnerability through a public GitHub issue, pull request, or discussion, in this repository or in `ownpg-releases`.

See the security policy in [devops-infinity/ownpg-releases](https://github.com/devops-infinity/ownpg-releases/security/policy) for the response-time commitments.

Include in your report:

- The affected version or commit.
- Steps to reproduce the issue.
- The impact: what an attacker gains, and under what conditions.

## Scope

A report is in scope when it shows:

- A destructive statement executing without passing through the confirmation gate (an explicit `confirm: true` argument or a sealed elicitation round trip).
- An authorization bypass in the HTTP bearer-token or OAuth layer, including a JWT accepted with the wrong audience, issuer, or expiration.
- A credential (a database password, an SSH key passphrase, a bearer token) appearing in a log, an audit-log entry, or a debug output.
- A TLS or SSH host-key verification bypass that the configured `sslmode` or SSH trust setting does not call for.

A report is out of scope when it describes a PostgreSQL role that the operator granted OwnPG more privilege than the task needs. OwnPG warns when it connects as a superuser or a role with `BYPASSRLS`, and refuses to start under `--strict-role`, but the grant itself is the operator's decision.

## Data OwnPG handles

OwnPG connects to one PostgreSQL database over TCP, a Unix socket, or an SSH tunnel, using a password, a `.pgpass` file, or an OS-keychain entry you provide. In HTTP mode, it validates a bearer token or a JWT against a JSON Web Key Set you configure. Its audit log records the statement class, a hash of the statement text, the calling principal, and the outcome of each call, and stays on the local machine. A short statement, 200 characters or fewer once normalized with literal values replaced by placeholders, is also kept in the clear alongside its hash. Run `ownpg man` for the audit-log path and the full environment-variable reference. Metrics export over OTLP stays off unless you set `OTEL_EXPORTER_OTLP_ENDPOINT`.
