# Security policy

## Supported versions

Only the most recent release receives security fixes. OwnPG is pre-1.0, so a fix ships as the next release, and earlier releases get no backports.

## Reporting a vulnerability

Report a vulnerability through GitHub's private vulnerability reporting on the release repository: https://github.com/devops-infinity/ownpg-releases/security/advisories/new. If that channel is unavailable, email sazzad@devops.bd instead. Do not report a vulnerability through a public GitHub issue, pull request, or discussion, in this repository or in `ownpg-releases`.

Expect acknowledgement within a few business days. Response time after that depends on severity and maintainer availability; OwnPG is pre-1.0 and does not commit to a fixed resolution deadline.

Include in your report:

- The affected version or commit.
- Steps to reproduce the issue.
- The impact: what an attacker gains, and under what conditions.

## Scope

A report is in scope when it affects OwnPG itself (the binary, the published crates, or a release artifact) and shows one of these:

- A statement running despite an access mode that does not allow it, such as a write or DDL statement under `--mode read-only`.
- A statement or a `DROP ... CASCADE` that reaches an object outside the served schema.
- A destructive statement executing without passing through the confirmation gate (an explicit `confirm: true` argument or a sealed elicitation round trip).
- An authorization bypass in the HTTP layer: a bearer token or JSON Web Token (JWT) accepted despite failing a check (a JWT with the wrong audience, issuer, or expiration, for example), `--auth none` accepted on an address outside the loopback interface, or a request served despite a `Host` or `Origin` outside the allowed list.
- A credential (a database password, an SSH key passphrase, a bearer token) appearing in a log, an audit-log entry, or a debug output.
- A TLS or SSH host-key verification bypass that the configured `sslmode` or SSH trust setting does not call for.
- A compromised release artifact, or a checksum or signature that does not match.

A report about the PostgreSQL server, role, or network you configured OwnPG to connect to is out of scope, and so is a role granted more privilege than the task needs. OwnPG warns when it connects as a superuser, an `rds_superuser` member, or a role with `BYPASSRLS`, and refuses to start under `--strict-role`, but the grant itself is the operator's decision.

## Data OwnPG handles

On the machine running OwnPG:

- Credentials come from a flag, an environment variable, a `.pgpass` file, the OS keychain, TLS client certificate and key files, and SSH keys or the SSH agent. Under the default `keychain_scope = "file"`, a keychain entry is bound to the profile file and the server it was saved for. In HTTP mode, bearer tokens are held as SHA-256 digests.
- The host-program tools pass the database password to `pg_dump` and the others through a temporary owner-only pgpass file in the data directory, never on the command line. A leftover file is removed after a day.
- A password given to `pg_role` is hashed with the server's `password_encryption` method before it is sent, unless it is already a SCRAM or MD5 hash.
- The audit log is a local file in the data directory that `ownpg config path` prints. Each line records the tool, the statement class, a fingerprint of the statement's structure (literal values left out), the relations it touches, the calling principal (a token name, or the JWT subject or client id, never the token itself), a server call id, and the outcome.
- The audit log also keeps two things in the clear: a statement of 200 bytes or less, normalized with literal values replaced by placeholders, and the full command line of a host-program tool, which never holds the password. It never keeps the text of a statement the parser can't read. Closed audit files are removed after 366 days unless `audit_keep_days` says otherwise, and each removal is recorded in the chain.
- `pg_activity`, `pg_locks`, and `pg_top_queries` show other sessions' statement text with every string and number literal replaced by `?`, and withhold a statement that mentions a password, secret, or credential.

Leaving the machine:

- SQL to the configured PostgreSQL server, and to PgBouncer's admin console for `pg_pool_status`.
- Tool results, to the connected MCP (Model Context Protocol) client.
- In OAuth mode, a request for the JSON Web Key Set at the configured URL.
- In HTTP mode, when `OTEL_EXPORTER_OTLP_ENDPOINT` or the `http.otel_endpoint` profile key is set, OpenTelemetry metrics: tool names, decisions, error codes, call durations, HTTP rejection reasons, audit-log health, connection-pool counts, the OwnPG version, and the database name. No statement text or principal is sent.
