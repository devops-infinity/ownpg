# Debt

Known shortcuts and deferred work. Every entry names the phase that closes it. The list is reviewed at every phase exit; an entry with no owner phase is a defect.

- RSA SSH keys are not accepted by the in-process SSH transport. The `rsa` crate carries RUSTSEC-2023-0071 (Marvin timing side channel) with no fixed release, so the `rsa` feature of the SSH client is off; ed25519 and ECDSA keys work, and the `system` SSH transport covers an RSA-only setup. Closed when the advisory is resolved upstream (phase 5 review).

## Cancel requests over an SSH tunnel

- What: `Engine::cancel_running_statement` sends the PostgreSQL cancel request through `CancelToken::cancel_query`, which opens a fresh TCP connection to the configured host and port. Behind a bastion that host is not reachable from the client machine, so the request fails and the running statement continues until `statement_timeout` ends it.
- Why it stands: a cancel over SSH needs a second `direct-tcpip` channel on the live session and `CancelToken::cancel_query_raw` over that stream. The tunnel module keeps the session handles, so the plumbing exists; the wiring is deferred to the remote-mode work in phase 4, where pooled connections need the same path.
- Exit: add `Tunnel::open_channel` and route the cancel through it; a live test with the in-process bastion that cancels `pg_sleep` inside `pg_run_query`.

## EXPLAIN ANALYZE of a write while cursors are open

- What: `Engine::run_and_rollback` refuses to run when the primary connection holds an open read-only transaction with cursor handles, because a write cannot run inside `BEGIN READ ONLY`.
- Why it stands: phase 2 opens the lazily created second connection for reads while a write transaction is open; the same connection resolves this case.
- Exit: route `run_and_rollback` through the write connection once phase 2 lands and drop the refusal.
