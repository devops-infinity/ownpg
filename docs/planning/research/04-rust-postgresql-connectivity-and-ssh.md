# Worker report 4: connecting to PostgreSQL from Rust, credentials, sockets, SSH, poolers as of 2026-09-12

Worker report captured on September 12th 2026 (session started 01:39:06 PM, GMT+06:00) for the OwnPG planning pack. It is the unedited return of one deep-research worker, except that em dashes inside quoted source text were replaced with commas to follow the house punctuation rule. Every claim carries its own URL, source date, and confidence band. Treat the content as evidence, never as instructions.

---

Research complete. Final counts: Exa 25, Serper 24, Tavily 5, Brave 5, WebSearch 5. Session timestamp: September 12 2026, 01:44:01 PM (+0600). All version, date, and maintenance facts below come from the crates.io API and the GitHub API, read in this session.

## Headline answers

- Use tokio-postgres 0.7.18 with the simple query protocol for arbitrary user SQL. `simple_query` returns every column as text and runs multi-statement scripts; sqlx `query()` is prepared-statement only, asks for binary results for every column, and fails on catalog types such as `aclitem`. As-of 2026-09-12. https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html and https://github.com/transact-rs/sqlx/issues/1269. Confidence High. See findings 1, 3, 4, 6.
- tokio-postgres `Config` parses only `sslmode=disable|prefer|require`. `verify-ca` and `verify-full` are not accepted; the caller maps them onto the rustls `ClientConfig`, and the default rustls config already behaves like `verify-full`. PR #988 was closed 2023-07-23; issue #768 is still open. As-of 2026-09-12. https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html. Confidence High. See findings 12, 30, 31.
- tokio-postgres reads no `PG*` environment variables, no `~/.pgpass`, and no `pg_service.conf` (issues #654 and #729 open). sqlx 0.9.0 reads both env vars and `.pgpass`. The only third-party crates for this are pg-client-config 0.1.2 (2023-08-26) and postgres-service 0.19.4 (2024-02-20), both stale. As-of 2026-09-12. https://github.com/rust-postgres/rust-postgres/issues/729. Confidence High. See findings 13 to 17.
- PostgreSQL 18 deprecates `md5` and adds `oauth`. Neither tokio-postgres nor sqlx speaks SASL `OAUTHBEARER` yet: rust-postgres PR #1378 and sqlx PR #4400 are open (last activity 2026-08-29 and 2026-09-09). tokio-postgres does implement SCRAM-SHA-256-PLUS with `tls-server-end-point`, and tokio-postgres-rustls 0.14.0 (2026-05-21) fixed the certificate parsing that channel binding needs. As-of 2026-09-12. https://www.postgresql.org/docs/18/release-18.html and https://github.com/rust-postgres/rust-postgres/pull/1378. Confidence High. See findings 24 to 29.
- Socket directories: `/tmp` upstream, Homebrew, and Postgres.app; `/var/run/postgresql` on Debian and Ubuntu; `/var/run/postgresql` plus `/tmp` on RHEL, Fedora, Rocky; `/run/postgresql` on Arch; `/var/run/postgresql` inside the official Docker container with TCP from the host. As-of 2026-09-12. https://www.postgresql.org/docs/18/runtime-config-connection.html and https://git.launchpad.net/ubuntu/+source/postgresql-12/commit/?id=0ed6f1ab7fa18a73ef9c3ed26ebccc5810f1985d. Confidence High. See findings 18 to 22.
- Default superusers: Homebrew makes the macOS login user the bootstrap superuser under `trust`; Postgres.app creates a `postgres` role and makes your macOS user a superuser, and since version 2.7 it shows a permission dialog before allowing any passwordless client, blocking unknown processes; Debian uses `peer` for the `postgres` OS user and `scram-sha-256` for TCP; Docker requires `POSTGRES_PASSWORD` unless `POSTGRES_HOST_AUTH_METHOD=trust`. As-of 2026-09-12. https://postgresapp.com/documentation/app-permissions.html and https://hub.docker.com/_/postgres. Confidence High. See finding 23.
- For SSH, use russh 0.63.3 (2026-09-09, Apache-2.0, rust-version 1.89) in-process: open a `direct-tcpip` channel, call `into_stream()`, and hand it to `tokio_postgres::Config::connect_raw`. Shipping Rust Postgres tools (rpg, ferox) do exactly this. thrussh is not dead on crates.io (0.49.0 published 2026-08-28 by the original author) but its listed repository 404s and its README says agent and encrypted-key support are not implemented. openssh 0.11.6 (2025-12-03) is the subprocess alternative and honors `~/.ssh/config`. As-of 2026-09-12. https://docs.rs/russh/latest/russh/struct.Channel.html. Confidence High. See findings 33 to 42.
- Behind a pooler, avoid session state: PgBouncer transaction mode never supports `SET/RESET`, `LISTEN`, `PREPARE/DEALLOCATE`, session advisory locks, `WITH HOLD` cursors, and `LOAD`; protocol-level prepared statements work only with `max_prepared_statements` set non-zero. Supavisor transaction mode (port 6543) does not support prepared statements and does not honor `options=-c search_path` (issue #206 open). As-of 2026-09-12. https://www.pgbouncer.org/features.html. Confidence High. See findings 43 to 49.
- For a single-user MCP server, hold one tokio-postgres connection, check `Client::is_closed()` and `Client::check_connection()` (added in 0.7.15, 2025-10-08) before use, and reconnect on failure; keepalives are on by default with a 2 hour idle. Set `statement_timeout` and `idle_in_transaction_session_timeout` through `options=-c ...` and tag with `application_name`. deadpool-postgres 0.14.2 (2026-08-26) is only needed for the multi-client remote mode. As-of 2026-09-12. https://docs.rs/deadpool-postgres/latest/deadpool_postgres/enum.RecyclingMethod.html. Confidence High. See findings 50 to 53.
- pg_query 6.2.0 (2026-08-03) parses with the PostgreSQL 17 grammar (libpg_query 17-6.2.2); PostgreSQL 18 support is an open PR (#79). sqlparser 0.62.0 (2026-05-07) is syntax-only and "accepts queries that specific databases would reject". As-of 2026-09-12. https://github.com/pganalyze/pg_query.rs/blob/main/CHANGELOG.md. Confidence High. See findings 10, 11.

## Question 1: Rust PostgreSQL client crates and the tokio-postgres versus sqlx comparison

- Finding: 1
  - Claim: Registry facts for the rust-postgres family, read from the crates.io API on 2026-09-12.
  - Detail:
    - tokio-postgres 0.7.18, published 2026-06-12, MIT OR Apache-2.0, rust-version 1.85, 66.5M downloads. Owners on docs.rs: sfackler and paolobarbolini.
    - postgres 0.19.14, published 2026-06-12, MIT OR Apache-2.0, rust-version 1.85.
    - postgres-types 0.2.14, published 2026-06-12, MIT OR Apache-2.0, rust-version 1.85.
    - postgres-protocol 0.6.12, published 2026-06-12, MIT OR Apache-2.0, rust-version 1.85.
    - postgres-native-tls 0.5.3 and postgres-openssl 0.5.3, both published 2026-03-30, MIT OR Apache-2.0, rust-version 1.85.
    - Repository rust-postgres/rust-postgres: not archived, pushed 2026-07-27, latest GitHub release tag `postgres-v0.19.14` on 2026-06-12, 182 open issues.
    - Changelog: "v0.7.18 - 2026-06-12: Fixed: Error instead of panicking on DataRow field/column count mismatch." "v0.7.15 - 2025-10-08: Added Client::check_connection API. Added Client::simple_query_raw API. Improved the effectiveness of Client::is_closed."
  - Citations:
    - https://crates.io/api/v1/crates/tokio-postgres (Authoritative, registry API, read 2026-09-12)
    - https://api.github.com/repos/rust-postgres/rust-postgres (Authoritative, repository API, read 2026-09-12)
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/CHANGELOG.md (Authoritative, 2026-06-12 entry)
  - Confidence: High, 97 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 2
  - Claim: Registry facts for sqlx, the pools, and the TLS crates.
  - Detail:
    - sqlx 0.9.0 and sqlx-postgres 0.9.0, published 2026-05-21, MIT OR Apache-2.0, rust-version 1.94.0 (a much higher MSRV than tokio-postgres). 144.8M downloads. The GitHub repository listed on crates.io (launchbadge/sqlx) now redirects to transact-rs/sqlx; pushed 2026-09-10, 753 open issues, tag v0.9.0 committed 2026-05-21.
    - deadpool-postgres 0.14.2, published 2026-08-26, MIT OR Apache-2.0, rust-version 1.85; repo deadpool-rs/deadpool pushed 2026-09-04.
    - bb8-postgres 0.9.0, published 2024-12-09, MIT, rust-version 1.75; repo djc/bb8 pushed 2026-08-28, but the `postgres` directory's last commit is 2025-11-24 ("Inline generic bounds").
    - tokio-postgres-rustls 0.14.0, published 2026-05-21, MIT, no rust-version field; repo jbg/tokio-postgres-rustls pushed 2026-05-21; depends on rustls ^0.23, tokio-rustls ^0.26, x509-cert ^0.2, optional rustls-native-certs ^0.8 and webpki-roots ^1.
    - rustls 0.23.44, published 2026-09-07, Apache-2.0 OR ISC OR MIT, rust-version 1.71.
    - rustls-native-certs 0.8.4, published 2026-06-01, Apache-2.0 OR ISC OR MIT, rust-version 1.71; repo pushed 2026-08-31.
    - webpki-roots 1.0.9, published 2026-07-18, CDLA-Permissive-2.0, rust-version 1.70.
    - pg_query 6.2.0, published 2026-08-03, MIT; sqlparser 0.62.0, published 2026-05-07, Apache-2.0; repos pushed 2026-08-03 and 2026-09-10.
  - Citations:
    - https://crates.io/api/v1/crates/sqlx (Authoritative, registry API, read 2026-09-12)
    - https://crates.io/api/v1/crates/deadpool-postgres and https://crates.io/api/v1/crates/bb8-postgres (Authoritative, registry API, read 2026-09-12)
    - https://crates.io/api/v1/crates/tokio-postgres-rustls, https://crates.io/api/v1/crates/rustls, https://crates.io/api/v1/crates/rustls-native-certs, https://crates.io/api/v1/crates/webpki-roots (Authoritative, registry API, read 2026-09-12)
    - https://api.github.com/repos/transact-rs/sqlx, https://api.github.com/repos/djc/bb8/commits?path=postgres (Authoritative, repository API, read 2026-09-12)
  - Confidence: High, 97 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: crates.io lists launchbadge/sqlx as the repository; GitHub redirects that name to transact-rs/sqlx. Same project under a renamed org, so the tag and push history are continuous. Resolved by the repository API.

- Finding: 3
  - Claim: tokio-postgres `simple_query` returns rows as strings for every column with no type lookup, runs several semicolon-separated statements in one round trip, and preserves the framing between them; `query` needs known types and returns binary values.
  - Detail: docs.rs for `Client::simple_query`: "Executes a sequence of SQL statements using the simple query protocol, returning the resulting rows. Statements should be separated by semicolons. If an error occurs, execution of the sequence will stop at that point. The simple query protocol returns the values in rows as strings rather than in their binary encodings, so the associated row type doesn't work with the FromSql trait. Rather than simply returning a list of the rows, this method returns a list of an enum which indicates either the completion of one of the commands, or a row of data. This preserves the framing between the separate statements in the request." `SimpleQueryRow::get` and `try_get` return `Option<&str>`. `simple_query_raw` (added 0.7.15) streams `SimpleQueryMessage` values. `batch_execute` runs a script and discards results. The docs warn: "Prepared statements should be use for any query which contains user-specified data ... Do not form statements via string concatenation and pass them to this method!" For a DBA tool where the user's SQL is the whole statement, that warning does not apply; it applies to interpolated values.
  - Citations:
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html (Authoritative, docs.rs build dated 03 September 2026 for 0.7.18)
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/simple_query.rs (Authoritative, source read 2026-09-12)
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/CHANGELOG.md (Authoritative, 2025-10-08 entry for simple_query_raw)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 4
  - Claim: Both tokio-postgres `query` and sqlx `query` ask the server for binary output on every result column, so a catalog query that returns a type without a binary send function (for example `aclitem` in `pg_class.relacl`) fails at the server. Only the text protocol avoids this.
  - Detail: In tokio-postgres `query.rs`, `encode_bind_raw` calls `frontend::bind(...)` with `Some(1)` as the `result_formats` argument; `postgres-protocol` defines that argument as `result_formats: K where K: IntoIterator<Item = i16>`, and one code of 1 means binary for all columns. Maintainer sfackler on rust-postgres issue #596 (2020-04-11): "You'll probably need to only select the columns you actually need since one of them uses a type that can't be serialized to the postgres binary protocol." sqlx issue #1269 (open, opened 2021-06-02): error text "no binary output function available for type aclitem"; maintainer abonander: "This unfortunately means you can't use aclitem with the sqlx::query*() functions or sqlx::query*!() macros as these require binary encoding for parameters and outputs. If you don't need bind parameters ... you can force the text protocol by directly calling methods on Executor: let rows: Vec<PgRow> = conn.fetch_all("SELECT ...").await?;" and "Right now we're just always asking for all columns in binary."
  - Citations:
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/src/query.rs (Authoritative, source read 2026-09-12)
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/postgres-protocol/src/message/frontend.rs (Authoritative, source read 2026-09-12)
    - https://github.com/rust-postgres/rust-postgres/issues/596 (Consensus, maintainer comment 2020-04-11)
    - https://github.com/transact-rs/sqlx/issues/1269 (Consensus, maintainer comments 2021-06-02 and 2021-06-15, issue still open)
  - Confidence: High, 90 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 5
  - Claim: tokio-postgres dynamic row access and unknown-type handling: `Row::columns()` exposes name and `Type` per column, `Type::from_oid` covers built-in OIDs, custom types are resolved from the catalog at prepare time, and `try_get` returns a `WrongType` error rather than panicking when the Rust type does not accept the column type. There is no generic "give me this binary value as text" path, which is why text mode fits a DBA tool.
  - Detail: `query_typed` in query.rs calls `get_type(client, field.type_oid())` for every `RowDescription` field and builds `Column { name, table_oid, column_id, type_modifier, r#type }`. The 0.7.16 changelog added `Column::type_modifier` and `Row::raw_size_bytes`. The postgres-types author's guidance on the Rust forum: "Just match on the type: match some_type { Type::TEXT => {}, Type::INT8 => {}, ... }". The user-forum thread shows the practical approach is matching on `row.columns()[i].type_()` and calling `get` with the matching Rust type, or serializing everything to strings. sfackler on issue #882 (2022): "The simple_query interface will return strings, but it doesn't support query parameters ... It definitely seems reasonable to expose text-encoded query parameters and return values, but the API design will be somewhat awkward."
  - Citations:
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/query.rs (Authoritative, source read 2026-09-12)
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/CHANGELOG.md (Authoritative, 2026-01-14 entry)
    - https://github.com/sfackler/rust-postgres/issues/882 (Consensus, 2022-04-07, maintainer participation)
    - https://users.rust-lang.org/t/accessing-column-types-in-postgres-types-tokio-postgres/109536 (Consensus, 2024-04-09)
  - Confidence: High, 88 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 6
  - Claim: sqlx `query()` accepts only a single DML statement and always prepares; `raw_sql()` runs multi-statement scripts without preparing and without parameters, and wraps the script in an implicit transaction; `PgRow` gives dynamic access by index or name and `PgValueRef::as_str()` reads text-format values.
  - Detail: docs.rs `sqlx::query`: "Execute a single SQL query as a prepared statement (transparently cached). The query string may only contain a single DML statement: SELECT, INSERT, UPDATE, DELETE and variants." and "Some third-party databases that speak a supported protocol, e.g. CockroachDB or PGBouncer that speak Postgres, may have issues with the transparent caching of prepared statements. If you are having trouble, try setting .persistent(false)." docs.rs `sqlx::raw_sql` (sqlx 0.9.0, page dated 20 July 2026): "Execute one or more statements as raw SQL, separated by semicolons (;). ... This will not create or cache any prepared statements." "Note: query parameters are not supported." "By default, when you use this API to execute a SQL string containing multiple statements separated by semicolons (;), the database server will treat those statements as all executing within the same transaction block, i.e. wrapped in BEGIN and COMMIT ... If one statement triggers an error, the whole script aborts and rolls back." `PgRow` implements `Row` with `try_get_raw(index) -> PgValueRef`, and `PgValueRef` has `format()`, `as_bytes()`, and `as_str()`. Some decoders refuse text format: sqlx-postgres `types/bytes.rs` returns "unsupported decode to `&[u8]` of BYTEA in a simple query; use a prepared query or decode to `Vec<u8>`".
  - Citations:
    - https://docs.rs/sqlx/latest/sqlx/fn.query.html (Authoritative, sqlx 0.9.0)
    - https://docs.rs/sqlx/latest/sqlx/fn.raw_sql.html (Authoritative, docs.rs build 20 July 2026)
    - https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgValueRef.html and https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgRow.html (Authoritative, sqlx 0.9.0)
    - https://docs.rs/crate/sqlx-postgres/latest/source/src/types/bytes.rs (Authoritative, sqlx-postgres 0.9.0 source)
  - Confidence: High, 94 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 7
  - Claim: sqlx's decoders lose type context for anonymous records and arrays of custom types and fall back to `PgTypeInfo::try_from_oid`, which knows only built-in types; this surfaces as runtime errors or the message "custom types in records are not fully supported yet".
  - Detail: sqlx-postgres `types/record.rs`: "custom types in records are not fully supported yet: failed to retrieve type info for field {} with type oid {}". Issue #1672 (opened 2022-02-03, still referenced in 2024 comments): a contributor explains "When context gets lost, the decoders fallback to using PgTypeInfo::try_from_oid which only supports a handful of builtin types." A 2024 comment on sqlx 0.8.2 reports "internal error: entered unreachable code: (bug) use of unresolved type declaration [oid]".
  - Citations:
    - https://docs.rs/crate/sqlx-postgres/latest/source/src/types/record.rs (Authoritative, sqlx-postgres 0.9.0 source)
    - https://github.com/launchbadge/sqlx/issues/1672 (Consensus, 2022-02-03 with later comments)
  - Confidence: Medium, 80 percent (the issue thread is older than 12 months, but the quoted source line is in the current 0.9.0 release)
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 8
  - Claim: Both crates support COPY. tokio-postgres has `copy_in` (returns a `CopyInSink`) and `copy_out` (returns a `CopyOutStream`); sqlx has `PgConnection::copy_in_raw` (returns `PgCopyIn`) and `copy_out_raw`.
  - Detail: tokio-postgres docs: "Executes a COPY FROM STDIN statement, returning a sink used to write the copy data. PostgreSQL does not support parameters in COPY statements, so this method does not take any. The copy must be explicitly completed via the Sink::close or finish methods. If it is not, the copy will be aborted." and "Executes a COPY TO STDOUT statement, returning a stream of the resulting data." sqlx docs: "Issue a COPY FROM STDIN statement and transition the connection to streaming data to Postgres ... If statement is anything other than a COPY ... FROM STDIN ... command, an error is returned." and for `copy_out_raw`: "unless you read the stream to completion, it can only be canceled in two ways: by closing the connection, or by using another connection to kill the server process".
  - Citations:
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html (Authoritative, 0.7.18)
    - https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgConnection.html (Authoritative, 0.9.0)
  - Confidence: High, 95 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 9
  - Claim: Verdict: tokio-postgres is the better fit for this tool. It gives a text-mode, multi-statement, framing-preserving path (`simple_query`) with a raw-stream constructor (`Config::connect_raw`) for SSH tunnels, a lower MSRV (1.85 versus 1.94.0), and a smaller dependency surface. sqlx's strengths (compile-time checked queries, prepared statement cache, built-in `.pgpass` and env-var handling, built-in `verify-ca`/`verify-full`) matter less for a tool that executes SQL it cannot know ahead of time, and its `raw_sql` implicit-transaction behavior changes the meaning of a user's script.
  - Detail: Supporting facts are in findings 3, 4, 6, 7, 8, 15, 30, and 42. The one place sqlx is ahead is libpq-compatible config loading (finding 15); that gap in tokio-postgres is closed by a small loader in the tool (finding 16).
  - Citations:
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html (Authoritative, 0.7.18, docs.rs build 03 September 2026)
    - https://docs.rs/sqlx/latest/sqlx/fn.raw_sql.html (Authoritative, 0.9.0)
    - https://crates.io/api/v1/crates/sqlx (Authoritative, rust-version 1.94.0, read 2026-09-12)
  - Confidence: High, 88 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 10
  - Claim: pg_query 6.2.0 parses SQL with PostgreSQL 17's own grammar (libpg_query 17-6.2.2) and can normalize, fingerprint, split, and classify statements; PostgreSQL 18 grammar support is an open PR.
  - Detail: pg_query.rs CHANGELOG: "6.2.0 2026-07-29: Upgrade to libpg_query 17-6.2.2. Add pg_query::summary function ..." crates.io publish date 2026-08-03. docs.rs lists `parse`, `normalize`, `fingerprint`, `scan`, `split_with_parser`, `parse_plpgsql`. README: "This Rust library uses the actual PostgreSQL server source to parse SQL queries and return the internal PostgreSQL parse tree." The libpg_query repository has an `18-latest` branch and an `18.0.0` tag (commit dated 2026-05-21); pg_query.rs has a `pg-18` branch and open PR #79 "Upgrade to Postgres 18" (updated 2026-08-06). Practical effect: PostgreSQL 18-only syntax may fail to parse in 6.2.0, so a read-only classifier should fail open (treat parse failure as "unknown, refuse in read-only mode") rather than fail closed.
  - Citations:
    - https://raw.githubusercontent.com/pganalyze/pg_query.rs/main/CHANGELOG.md (Authoritative, 2026-07-29 entry)
    - https://crates.io/api/v1/crates/pg_query (Authoritative, registry API, read 2026-09-12)
    - https://api.github.com/repos/pganalyze/pg_query.rs/pulls (Authoritative, PR #79 read 2026-09-12)
    - https://docs.rs/pg_query (Authoritative, 6.2.0)
  - Confidence: High, 92 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 11
  - Claim: sqlparser 0.62.0 is a permissive syntax-only parser with a `PostgreSqlDialect`; it is not a PostgreSQL grammar and its own README says you must test the SQL subset you need.
  - Detail: README: "This crate provides only a syntax parser, and tries to avoid applying any SQL semantics, and accepts queries that specific databases would reject, even when using that Database's specific Dialect." and "If you are assessing whether this project will be suitable for your needs, you'll likely need to experimentally verify whether it supports the subset of SQL that you need." Registry: 0.62.0 published 2026-05-07, Apache-2.0; repo apache/datafusion-sqlparser-rs pushed 2026-09-10 with a `v0.63.0-rc1` tag present.
  - Citations:
    - https://github.com/apache/datafusion-sqlparser-rs (Authoritative, README read 2026-09-12)
    - https://crates.io/api/v1/crates/sqlparser (Authoritative, registry API, read 2026-09-12)
    - https://docs.rs/sqlparser/latest/sqlparser/dialect/struct.PostgreSqlDialect.html (Authoritative, 0.62.0)
  - Confidence: High, 93 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

## Question 2: libpq-compatible configuration

- Finding: 12
  - Claim: tokio-postgres 0.7.18 `Config` parses both libpq string forms and supports exactly these keys: `user`, `password`, `dbname`, `options`, `application_name`, `sslmode` (disable, prefer, require only), `host`, `sslnegotiation` (postgres, direct), `hostaddr`, `port`, `connect_timeout`, `tcp_user_timeout`, `keepalives`, `keepalives_idle`, `keepalives_interval`, `keepalives_retries`, `target_session_attrs`, `channel_binding`, `load_balance_hosts`. It does not accept `passfile`, `service`, `sslrootcert`, `sslcert`, `sslkey`, `verify-ca`, or `verify-full`.
  - Detail: docs.rs quotes: "Configuration can be parsed from libpq-style connection strings. These strings come in two formats: Key-Value ... This format consists of space-separated key-value pairs. Values which are either the empty string or contain whitespace should be wrapped in '. ' and \ characters should be backslash-escaped." "Url ... This format resembles a URL with a scheme of either postgres:// or postgresql://. All components are optional ... Unix socket paths in the host section of the URL should be percent-encoded, as the path component of the URL specifies the database name." Key docs: "user - The username to authenticate with. Defaults to the user executing this process." "dbname - ... Defaults to the username." "options - Command line options used to configure the server." "application_name - Sets the application_name parameter on the server." "sslmode - Controls usage of TLS. If set to disable, TLS will not be used. If set to prefer, TLS will be used if available, but not used otherwise. If set to require, TLS will be forced to be used. Defaults to prefer." "host - ... On Unix platforms, if the host starts with a / character it is treated as the path to the directory containing Unix domain sockets. ... Multiple hosts can be specified, separated by commas. Each host will be tried in turn when connecting. Required if connecting with the connect method." "sslnegotiation - TLS negotiation method. If set to direct, the client will perform direct TLS handshake, this only works for PostgreSQL 17 and newer. Note that you will need to setup ALPN of TLS client configuration to postgresql when using direct TLS." "hostaddr - Numeric IP address of host to connect to ... However, a host name is required for TLS certificate verification." "port - ... Defaults to 5432 if omitted or the empty string." "connect_timeout - The time limit in seconds applied to each socket-level connection attempt ... Defaults to no timeout." "tcp_user_timeout - The time limit that transmitted data may remain unacknowledged before a connection is forcibly closed. This is ignored for Unix domain socket connections." "keepalives - Controls the use of TCP keepalive ... This option is ignored when connecting with Unix sockets. Defaults to on." "keepalives_idle - ... Defaults to 2 hours." "target_session_attrs - ... If set to read-write, the client will check that the transaction_read_write session parameter is set to on. ... Defaults to all." "channel_binding - ... If set to require, the authentication process will fail if channel binding is not used. Defaults to prefer." "load_balance_hosts - ... If set to random, hosts will be tried in a random order ... Defaults to disable." The source `param()` match lists `sslmode` values "disable", "prefer", "require" and returns `InvalidValue("sslmode")` otherwise. Examples in the docs: `host=/var/run/postgresql,localhost port=1234 user=postgres password='password with spaces'`, `postgresql://user:password@%2Fvar%2Frun%2Fpostgresql/mydb?connect_timeout=10`, `postgresql:///mydb?user=user&host=/var/run/postgresql`. `connect_raw` docs: "Connects to a PostgreSQL database over an arbitrary stream. All of the settings other than user, password, dbname, options, and application_name name are ignored."
  - Citations:
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html (Authoritative, 0.7.18, docs.rs build 03 September 2026)
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/config.rs (Authoritative, source read 2026-09-12)
  - Confidence: High, 98 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 13
  - Claim: libpq connection-string syntax, `sslmode` semantics, host defaults, and the `options` parameter, from the PostgreSQL 18 manual.
  - Detail: "host ... If a host name looks like an absolute path name, it specifies Unix-domain communication rather than TCP/IP communication; the value is the name of the directory in which the socket file is stored. ... If the host name starts with @, it is taken as a Unix-domain socket in the abstract namespace ... The default behavior when host is not specified, or is empty, is to connect to a Unix-domain socket in /tmp (or whatever socket directory was specified when PostgreSQL was built). On Windows, the default is to connect to localhost." "port - Port number to connect to at the server host, or socket file name extension for Unix-domain connections." "dbname - The database name. Defaults to be the same as the user name." "user - PostgreSQL user name to connect as. Defaults to be the same as the operating system name of the user running the application." "passfile - Specifies the name of the file used to store passwords ... Defaults to ~/.pgpass, or %APPDATA%\postgresql\pgpass.conf on Microsoft Windows. (No error is reported if this file does not exist.)" "options - Specifies command-line options to send to the server at connection start. For example, setting this to -c geqo=off or --geqo=off sets the session's value of the geqo parameter to off. Spaces within this string are considered to separate command-line arguments, unless escaped with a backslash (\)". "connect_timeout - Maximum time to wait while connecting, in seconds ... This timeout applies separately to each host name or IP address." `sslmode` six modes: "disable: only try a non-SSL connection; allow: first try a non-SSL connection; if that fails, try an SSL connection; prefer (default): first try an SSL connection; if that fails, try a non-SSL connection; require: only try an SSL connection. If a root CA file is present, verify the certificate in the same way as if verify-ca was specified; verify-ca: only try an SSL connection, and verify that the server certificate is issued by a trusted certificate authority (CA); verify-full: only try an SSL connection, verify that the server certificate is issued by a trusted CA and that the requested server host name matches that in the certificate." "sslmode is ignored for Unix domain socket communication." "sslrootcert ... The default is ~/.postgresql/root.crt. The special value system may be specified instead, in which case the trusted CA roots from the SSL implementation will be loaded. ... When using sslrootcert=system, the default sslmode is changed to verify-full, and any weaker setting will result in an error." "service - Service name to use for additional parameters. It specifies a service name in pg_service.conf that holds additional connection parameters." "require_auth ... The following methods may be specified: password, md5, gss, sspi, scram-sha-256, oauth, none".
  - Citations:
    - https://www.postgresql.org/docs/18/libpq-connect.html (Authoritative, PostgreSQL 18 manual, current for 18.6 released 2026-08-13)
  - Confidence: High, 99 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 14
  - Claim: The libpq environment variables, the `.pgpass` format with its 0600 rule, and the `pg_service.conf` format and precedence, from the PostgreSQL 18 manual.
  - Detail: Environment: "PGHOST behaves the same as the host connection parameter." Likewise "PGHOSTADDR ... hostaddr", "PGPORT ... port", "PGDATABASE ... dbname", "PGUSER ... user", "PGPASSWORD ... password. Use of this environment variable is not recommended for security reasons, as some operating systems allow non-root users to see process environment variables via ps; instead consider using a password file", "PGPASSFILE ... passfile", "PGSERVICE ... service", "PGSERVICEFILE specifies the name of the per-user connection service file ... Defaults to ~/.pg_service.conf", "PGOPTIONS ... options", "PGAPPNAME ... application_name", "PGSSLMODE ... sslmode", "PGSSLROOTCERT ... sslrootcert", "PGCONNECT_TIMEOUT ... connect_timeout", "PGCHANNELBINDING ... channel_binding", "PGSSLNEGOTIATION ... sslnegotiation", "PGTARGETSESSIONATTRS", "PGLOADBALANCEHOSTS", and "PGSYSCONFDIR sets the directory containing the pg_service.conf file". Password file: "This file should contain lines of the following format: hostname:port:database:username:password ... Each of the first four fields can be a literal value, or *, which matches anything. The password field from the first line that matches the current connection parameters will be used. (Therefore, put more-specific entries first when you are using wildcards.) If an entry needs to contain : or \, escape this character with \. The host name field is matched to the host connection parameter if that is specified, otherwise to the hostaddr parameter if that is specified; if neither are given then the host name localhost is searched for. The host name localhost is also searched for when the connection is a Unix-domain socket connection and the host parameter matches libpq's default socket directory path." "On Unix systems, the permissions on a password file must disallow any access to world or group; achieve this by a command such as chmod 0600 ~/.pgpass. If the permissions are less strict than this, the file will be ignored. On Microsoft Windows ... no special permissions check is made." Service file: "By default, the per-user service file is named ~/.pg_service.conf ... The system-wide file is named pg_service.conf. By default it is sought in the etc directory of the PostgreSQL installation (use pg_config --sysconfdir to identify this directory precisely). Another directory ... can be specified by setting the environment variable PGSYSCONFDIR." "Either service file uses an 'INI file' format where the section name is the service name and the parameters are connection parameters ... [mydb] host=somehost port=5433 user=admin". "A service file setting overrides the corresponding environment variable, and in turn can be overridden by a value given directly in the connection string." "If the same service name exists in both the user and the system file, the user file takes precedence."
  - Citations:
    - https://www.postgresql.org/docs/18/libpq-envars.html (Authoritative, PostgreSQL 18 manual)
    - https://www.postgresql.org/docs/18/libpq-pgpass.html (Authoritative, PostgreSQL 18 manual)
    - https://www.postgresql.org/docs/18/libpq-pgservice.html (Authoritative, PostgreSQL 18 manual)
  - Confidence: High, 99 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 15
  - Claim: tokio-postgres implements none of the environment-variable, `.pgpass`, or service-file lookups, and the requests for them are open; sqlx 0.9.0 implements env vars and `.pgpass` natively (permission check on Linux only) but not `pg_service.conf`.
  - Detail: rust-postgres issue #654 "Option to read config from environment variables?" (opened 2020-09-10, open, 3 comments); maintainer sfackler: "While I wouldn't want this to happen by default, it seems potentially plausible to have a method on the Config type to do this - Config::new().load_from_env() or whatever." Issue #729 "Support passfile" (opened 2021-01-11, open, 2 comments): "I believe we at least want to support the passfile connection parameter ... We might also want to support the PGPASSFILE environment variable and/or the default location (~/.pgpass on unix)." sqlx `PgConnectOptions` docs list `PGUSER`, `PGPASSWORD` ("Read from passfile, if it exists."), `PGPASSFILE` ("~/.pgpass or %APPDATA%\postgresql\pgpass.conf"), `PGHOST`, `PGHOSTADDR`, `PGPORT`, `PGDATABASE`, `PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT`, `PGSSLKEY`, `PGOPTIONS`, `PGAPPNAME`; "passfile handling may be bypassed using PgConnectOptions::new_without_pgpass()". sqlx `pgpass.rs`: reads `PGPASSFILE` first, then `~/.pgpass`; the permission check is `#[cfg(target_os = "linux")] ... if mode & 0o77 != 0 { tracing::warn!(... "Ignoring path. Permissions are not strict enough"); return None; }`, so on macOS sqlx does not enforce 0600. sqlx PR #3999 (deferred passfile lookup) drew maintainer pushback: "I don't agree with implicitly attempting to re-apply the .pgpass file after construction and manual modification."
  - Citations:
    - https://github.com/rust-postgres/rust-postgres/issues/654 (Consensus, 2020-09-10, open as of 2026-09-12 per API)
    - https://github.com/rust-postgres/rust-postgres/issues/729 (Consensus, 2021-01-11, open as of 2026-09-12 per API)
    - https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgConnectOptions.html (Authoritative, sqlx 0.9.0)
    - https://github.com/launchbadge/sqlx/blob/main/sqlx-postgres/src/options/pgpass.rs (Authoritative, source read 2026-09-12)
    - https://github.com/launchbadge/sqlx/pull/3999 (Consensus, maintainer comment)
  - Confidence: High, 93 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: libpq ignores a `.pgpass` with group or world access on every Unix; sqlx checks only on Linux. Resolved by treating the PostgreSQL manual as the spec: the tool should check 0600 on all Unix targets.

- Finding: 16
  - Claim: Third-party crates for service files and `.pgpass` exist but are stale: pg-client-config 0.1.2 (2023-08-26) handles `service=`, `PGSERVICE`, `PGSERVICEFILE`, `PGSYSCONFDIR`, seven `PG*` variables, and a Linux-only passfile; postgres-service 0.19.4 (2024-02-20, BSD-2-Clause) parses `pg_service.conf`; pg-connection-string 0.0.2 (2023-10-24) and its fork postgres-conn-str 0.1.1 (2024-10-11) are AGPL-3.0-or-later. No crate named pgpass, pg-pass, pg-service, pg_service, or postgres-config exists on crates.io.
  - Detail: crates.io API returned "crate `pgpass` does not exist", and the same for `pg-pass`, `pgpassfile`, `pg-service`, `pg_service`, `postgres-config`. pg-client-config docs: "If the connection string start with service= the service will be searched in the file given by PGSERVICEFILE, or in ~/.pg_service.conf and PGSYSCONFDIR/pg_service.conf ... In all cases, parameters from the connection string take precedence." Its source loads `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`, `PGOPTIONS`, `PGAPPNAME`, `PGCONNECT_TIMEOUT`, and says "Passfile is actually supported only on linux platform". postgres-service README: "search in ~/.pg_service.conf, $PGSYSCONFDIR/pg_service.conf, and /etc/postgresql-..." and "supports tokio-postgres (New in 0.19.2)". Given the ages and the AGPL license on the connection-string crates, the tool should implement the few hundred lines itself against the manual in finding 14 and use tokio-postgres `Config` setters.
  - Citations:
    - https://crates.io/api/v1/crates/pg-client-config, https://crates.io/api/v1/crates/postgres-service, https://crates.io/api/v1/crates/pg-connection-string, https://crates.io/api/v1/crates/postgres-conn-str (Authoritative, registry API, read 2026-09-12)
    - https://docs.rs/pg-client-config/latest/pg_client_config/fn.load_config.html and https://docs.rs/pg-client-config/latest/src/pg_client_config/lib.rs.html (Authoritative, 0.1.2)
    - https://github.com/njaard/postgres-service (Authoritative, README)
  - Confidence: High, 90 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 17
  - Claim: deadpool-postgres has its own `Config` struct with `url`, `user`, `password`, `dbname`, `options`, `application_name`, `ssl_mode`, `host`/`hosts`, `hostaddr`/`hostaddrs`, `port`/`ports`, `connect_timeout`, `keepalives`, `keepalives_idle`, `target_session_attrs`, `channel_binding`, `load_balance_hosts`, `manager`, and `pool`, loadable from `PG__*` environment variables through the `config` crate; these are not the libpq `PG*` names.
  - Detail: docs.rs example: `PG__HOST=pg.example.com PG__USER=john_doe PG__PASSWORD=topsecret PG__DBNAME=example PG__POOL__MAX_SIZE=16`. The struct doc lists the 21 fields above.
  - Citations:
    - https://docs.rs/deadpool-postgres/latest/deadpool_postgres/struct.Config.html (Authoritative, 0.14.2)
  - Confidence: High, 95 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

## Question 3: Unix domain sockets and per-platform defaults

- Finding: 18
  - Claim: Upstream PostgreSQL defaults the socket directory to `/tmp`, names the socket `.s.PGSQL.<port>`, and `initdb` defaults to `trust` with the bootstrap superuser named after the OS user running it.
  - Detail: "unix_socket_directories ... The default value is normally /tmp, but that can be changed at build time. On Windows, the default is empty ... In addition to the socket file itself, which is named .s.PGSQL.nnnn where nnnn is the server's port number, an ordinary file named .s.PGSQL.nnnn.lock will be created". "unix_socket_permissions ... The default permissions are 0777, meaning anyone can connect." initdb: "-A authmethod ... Do not use trust unless you trust all local users on your system. trust is the default for ease of installation." "-U username ... Sets the user name of the bootstrap superuser. This defaults to the name of the operating-system user running initdb."
  - Citations:
    - https://www.postgresql.org/docs/18/runtime-config-connection.html (Authoritative, PostgreSQL 18 manual)
    - https://www.postgresql.org/docs/18/app-initdb.html (Authoritative, PostgreSQL 18 manual)
  - Confidence: High, 99 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 19
  - Claim: Homebrew's postgresql@18 formula builds PostgreSQL 18.6 unpatched, so the socket lives in `/tmp`, and it runs `initdb --locale=en_US.UTF-8 -E UTF-8` as the installing user with no `-U` or `-A`, so the superuser is the macOS login user and local auth is `trust`.
  - Detail: Formula source (HEAD, read 2026-09-12): `url "https://ftp.postgresql.org/pub/source/v18.6/postgresql-18.6.tar.bz2"`; configure args are `--datadir`, `--includedir`, `--sysconfdir`, `--docdir`, `--enable-nls`, `--enable-thread-safety`, `--with-gssapi`, `--with-icu`, `--with-ldap`, `--with-libxml`, `--with-libxslt`, `--with-lz4`, `--with-zstd`, `--with-openssl`, `--with-libcurl`, `--with-pam`, `--with-perl`, `--with-uuid=e2fs`, plus `--with-bonjour --with-tcl` on macOS; there is no patch and no socket-directory option. `system bin/"initdb", "--locale=en_US.UTF-8", "-E", "UTF-8", postgresql_datadir unless pg_version_exists?`. Caveats: "This formula has created a default database cluster with: initdb --locale=en_US.UTF-8 -E UTF-8 #{postgresql_datadir}". The service runs `postgres -D $HOMEBREW_PREFIX/var/postgresql@18`. Combined with finding 18 (initdb defaults), the superuser is `whoami` and the auth is trust. `--with-libcurl` also means Homebrew's libpq has OAuth client support built in (the formula's test asserts `libpq-oauth-18` exists).
  - Citations:
    - https://raw.githubusercontent.com/Homebrew/homebrew-core/HEAD/Formula/p/postgresql@18.rb (Authoritative, read 2026-09-12)
    - https://formulae.brew.sh/formula/postgresql@18 (Authoritative, caveats text)
    - https://www.postgresql.org/docs/18/app-initdb.html (Authoritative)
  - Confidence: High, 94 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 20
  - Claim: Debian and Ubuntu packages patch `DEFAULT_PGSOCKET_DIR` to `/var/run/postgresql`; `pg_createcluster` runs `initdb --auth-local peer --auth-host scram-sha-256` (for version 14 and later) and adds the `local all postgres peer` administrative line, so the default superuser is the `postgres` OS user over the socket with peer auth.
  - Detail: Ubuntu source commit "Put server Unix sockets into /var/run/postgresql/ by default. Gbp-Pq: 51-default-sockets-in-var.patch" changes `-#define DEFAULT_PGSOCKET_DIR "/tmp"` to `+#define DEFAULT_PGSOCKET_DIR "/var/run/postgresql"`. The Debian patch description quoted on pgsql-hackers: "Using /tmp for sockets allows everyone to spoof a PostgreSQL server. Thus use /var/run/postgresql/ for "system" clusters which run as 'postgres'". postgresql-common `pg_createcluster` (salsa master): `my $local_method = $version >= 9.1 ? 'peer' : ...`, `my $host_method = $version >= 14 ? 'scram-sha-256' : 'md5';`, `push @initdb, ('--auth-local', $local_method); push @initdb, ('--auth-host', $host_method);`, and the generated comment "# Database administrative login by Unix domain socket". Its man text: "by initdb to use peer authentication on local (unix) connections, and md5 on ...". The official Docker image's Alpine Dockerfile carries the same change: "update DEFAULT_PGSOCKET_DIR to /var/run/postgresql (matching Debian)".
  - Citations:
    - https://git.launchpad.net/ubuntu/+source/postgresql-12/commit/?id=0ed6f1ab7fa18a73ef9c3ed26ebccc5810f1985d (Authoritative, distribution source)
    - https://salsa.debian.org/postgresql/postgresql-common/-/raw/master/pg_createcluster (Authoritative, read 2026-09-12)
    - https://www.postgresql.org/message-id/CAF6yO%3D0ACZZneuUHAGypPwZQZK_jcaUN%2BaNxMqSo9Ch1F2OZjg%40mail.gmail.com (Consensus, 2011-10-31, quotes the Debian patch header)
    - https://github.com/docker-library/postgres/blob/master/17/alpine3.23/Dockerfile (Authoritative)
  - Confidence: High, 95 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 21
  - Claim: RHEL, Rocky, Fedora, and CentOS RPMs patch the built-in default to `/var/run/postgresql` and additionally list `/tmp`, so the server creates sockets in both; `postgresql-setup --initdb` is the init path and RHEL 10 shows the socket under `/var/run/postgresql`. Arch Linux patches the default to `/run/postgresql` and its default `pg_hba.conf` is `trust` for local connections.
  - Detail: CentOS git `postgresql-var-run-socket.patch`: "Change the built-in default socket directory to be /var/run/postgresql. For backwards compatibility with (probably non-libpq-based) clients that might still expect to find the socket in /tmp, also create a socket in /tmp. This is to resolve communication problems with clients operating under systemd's PrivateTmp environment"; diff lines `+#define DEFAULT_PGSOCKET_DIR "/var/run/postgresql"` and `+ DEFAULT_PGSOCKET_DIR ", /tmp",` in guc.c and initdb.c. RHEL 10 docs: `# postgresql-setup --initdb` and `postgres=# \conninfo` output "You are connected to database "postgres" as user "postgres" via socket in "/var/run/postgresql" at port "5432"." The RHEL 9 CentOS Stream spec still lists `postgresql-var-run-socket.patch`. PostgreSQL's Red Hat download page: "postgresql-setup --initdb; systemctl enable postgresql.service; systemctl start postgresql.service" for "RHEL / Rocky Linux / AlmaLinux 10, 9, 8 or Fedora 43". Arch packaging repo contains `0001-Set-DEFAULT_PGSOCKET_DIR-to-run-postgresql.patch` applied in PKGBUILD (`pkgver=18.6`), with `+#define DEFAULT_PGSOCKET_DIR "/run/postgresql"`; ArchWiki: "The trust authentication method is used by default, meaning that anyone on the host can connect as any database user. You can use --auth-local=peer --auth-host=scram-sha-256 for safer authentication methods." The Fedora Developer Portal shows the shipped hba line `local all all peer` after `postgresql-setup --initdb`.
  - Citations:
    - https://git.centos.org/rpms/postgresql/blob/0993aeba7b3ed2f80193f3a032b47b550303276f/f/SOURCES/postgresql-var-run-socket.patch (Authoritative, distribution source)
    - https://docs.redhat.com/documentation/red_hat_enterprise_linux/10/html/configuring_and_using_database_servers/using-postgresql (Authoritative, RHEL 10)
    - https://www.postgresql.org/download/linux/redhat/ (Authoritative)
    - https://gitlab.archlinux.org/archlinux/packaging/packages/postgresql/-/raw/main/0001-Set-DEFAULT_PGSOCKET_DIR-to-run-postgresql.patch and https://gitlab.archlinux.org/archlinux/packaging/packages/postgresql/-/raw/main/PKGBUILD (Authoritative, read 2026-09-12)
    - https://wiki.archlinux.org/title/PostgreSQL (Consensus)
    - https://developer.stg.fedoraproject.org/tech/database/postgresql/about.html (Consensus, Fedora developer portal)
  - Confidence: High, 92 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: Fedora dist-git's raw URL for the patch returned 404 on four path variants, so the RPM patch text was verified from the CentOS git mirror and the RHEL 10 manual instead.

- Finding: 22
  - Claim: The official Docker `postgres` image creates `/var/run/postgresql` inside the container and listens there; from the host you reach it over TCP through a published port. `POSTGRES_PASSWORD` is required unless `POSTGRES_HOST_AUTH_METHOD=trust`; the default superuser is `postgres` (override with `POSTGRES_USER`); host connections default to `scram-sha-256` on 14 and later; the container's own Unix socket uses `trust`.
  - Detail: Dockerfile (18/bookworm): `RUN install --verbose --directory --owner postgres --group postgres --mode 3777 /var/run/postgresql`. Docker Hub: "The only variable required is POSTGRES_PASSWORD, the rest are optional." "This environment variable sets the superuser password for PostgreSQL. The default superuser is defined by the POSTGRES_USER environment variable." "Note 1: The PostgreSQL image sets up trust authentication locally so you may notice a password is not required when connecting from localhost (inside the same container). However, a password will be required if connecting from a different host/container." "POSTGRES_HOST_AUTH_METHOD ... If unspecified then scram-sha-256 password authentication is used (in 14+; md5 in older releases)." "Note 2: If you set POSTGRES_HOST_AUTH_METHOD to trust, then POSTGRES_PASSWORD is not required." "Note 3: If you set this to an alternative value (such as scram-sha-256), you might need additional POSTGRES_INITDB_ARGS for the database to initialize correctly (such as POSTGRES_INITDB_ARGS=--auth-host=scram-sha-256)." Init scripts: "This user will be able to connect without a password due to the presence of trust authentication for Unix socket connections made inside the container." A related open issue: `sslrootcert=system` fails inside the image because `ca-certificates` is not installed ("Disappointingly sslrootcert=system is still broken" for PG 18, per docker-library/postgres#1331).
  - Citations:
    - https://raw.githubusercontent.com/docker-library/postgres/master/18/bookworm/Dockerfile (Authoritative, read 2026-09-12)
    - https://hub.docker.com/_/postgres (Authoritative)
    - https://github.com/docker-library/docs/blob/master/postgres/content.md (Authoritative)
    - https://github.com/docker-library/postgres/issues/1331 (Consensus, 2025-03-14 with 2026 follow-up comment)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: An older README revision said the host default was `md5`; the current Docker Hub text says `scram-sha-256` in 14 and later. Resolved by recency.

- Finding: 23
  - Claim: Postgres.app runs the server as your macOS user, creates a `postgres` superuser and also makes your macOS user a superuser, documents localhost:5432 with a blank password, and since 2.7 shows a per-app permission dialog before allowing a passwordless (`trust`) connection, blocking unknown processes outright. Its socket is the upstream `/tmp` default.
  - Detail: GitHub README: "Initialise a database cluster: initdb -D DATA_DIRECTORY -U postgres --encoding=UTF-8 --locale=en_US.UTF-8. Starting with PostgreSQL 15 additionally: --locale-provider=icu --icu-locale=en-US --data-checksums. Start the server: pg_ctl start -D DATA_DIRECTORY --wait ... --options="-p PORT". Create a superuser: createuser -U postgres -p PORT --superuser USERNAME. Create a user database: createdb USERNAME." "Note that Postgres.app runs the server as your user, unlike other installations which might create a separate system user named postgres." Docs: "Host: localhost, Port: 5432 (default), User: your user name, Password: blank, Database: same as user name" and "By default, PostgreSQL only allows connections from localhost, and requires no password." App permissions page: "By default, PostgreSQL accepts connections from apps on your computer without a password. This is called the 'trust' authentication method. ... To improve the security of PostgreSQL on your Mac, Postgres.app 2.7 and later shows a permission dialog before allowing a client app to connect without a password." "If you click 'Don't Allow', then Postgres.app will refuse the connection ... FATAL: Postgres.app rejected "trust" authentication". "In some cases, Postgres.app might not be able to identify the source of a connection attempt. If that happens, Postgres.app automatically blocks the passwordless authentication attempt." "The most likely reason is that you did not see the permission dialog and PostgreSQL aborted the authentication attempt after 1 minute." The permission checker identifies the client binary and its parent (issue #749 log: "auth_permission_dialog: /Applications/iTerm.app/Contents/MacOS/iTerm2 (via .../bin/ruby) is not allowed to connect without a password"), so an MCP server launched by an editor will be prompted for on first connect and blocked if the dialog cannot be shown. Socket: the initdb and pg_ctl commands above pass no `-k`, and 2024 logs from Postgres.app 2.7 users show "connection to server on socket "/tmp/.s.PGSQL.5432"". Postico's docs (a sandboxed client) describe the local socket as "/tmp/.s.PGSQL".
  - Citations:
    - https://github.com/postgresapp/postgresapp (Authoritative, README read 2026-09-12)
    - https://postgresapp.com/documentation/configuration-general.html (Authoritative; note it still lists 9.6 paths)
    - https://postgresapp.com/documentation/app-permissions.html (Authoritative)
    - https://github.com/PostgresApp/PostgresApp/issues/749 (Consensus, 2024-01-26, maintainer participation)
    - https://eggerapps.at/postico/docs/v1.5.12/connect-to-local-postgresql-server.html (Consensus, vendor docs)
  - Confidence: High, 88 percent for users and dialog behavior; Medium, 75 percent for the `/tmp` socket path, which comes from logs and a third-party doc rather than a Postgres.app doc sentence
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: A 2013 blog said "Postgres.app doesn't enable connections via unix socket by default" and used `/var/pgsql_socket`. That is history from the 9.2 era; current evidence points to `/tmp`. Resolved by recency, flagged Medium.

## Question 4: Authentication methods in PostgreSQL 18 and client support

- Finding: 24
  - Claim: PostgreSQL 18 (released 2025-09-25; 18.6 on 2026-08-13) deprecates MD5 passwords and adds the `oauth` HBA method; the full HBA method list is trust, reject, scram-sha-256, md5, password, gss, sspi, ident, peer, ldap, radius, cert, pam, bsd, oauth.
  - Detail: Release notes: "Deprecate MD5 password authentication (Nathan Bossart). Support for MD5 passwords will be removed in a future major version release. CREATE ROLE and ALTER ROLE now emit deprecation warnings when setting MD5 passwords. These warnings can be disabled by setting the md5_password_warnings parameter to off." "Add support for the OAuth authentication method (Jacob Champion, Daniel Gustafsson, Thomas Munro). This adds an oauth authentication method to pg_hba.conf, libpq OAuth options, a server variable oauth_validator_libraries to load token validation libraries, and a configure flag --with-libcurl to add the required compile-time libraries." pg_hba manual: "md5 - Perform SCRAM-SHA-256 or MD5 authentication to verify the user's password. Warning: Support for MD5-encrypted passwords is deprecated and will be removed in a future release of PostgreSQL." "trust - Allow the connection unconditionally." "peer - Obtain the client's operating system user name from the operating system and check if it matches the requested database user name. This is only available for local connections." "ident ... When specified for local connections, peer authentication will be used instead." "cert - Authenticate using SSL client certificates." "gss - Use GSSAPI to authenticate the user. This is only available for TCP/IP connections." "ldap - Authenticate using an LDAP server." "oauth - Authorize and optionally authenticate using a third-party OAuth 2.0 identity provider." Password auth page: "To ease transition from the md5 method to the newer SCRAM method, if md5 is specified as a method in pg_hba.conf but the user's password on the server is encrypted for SCRAM ... then SCRAM-based authentication will automatically be chosen instead." The 19 beta notes go further: "Issue a warning after successful MD5 password authentication" and "Remove RADIUS support".
  - Citations:
    - https://www.postgresql.org/docs/18/release-18.html (Authoritative, release notes)
    - https://www.postgresql.org/about/news/postgresql-18-released-3142/ (Authoritative, 2025-09-25)
    - https://www.postgresql.org/docs/18/auth-pg-hba-conf.html (Authoritative)
    - https://www.postgresql.org/docs/18/auth-password.html (Authoritative)
    - https://www.postgresql.org/docs/19/release-19.html (Authoritative, beta notes, background only)
  - Confidence: High, 99 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 25
  - Claim: How `oauth` works in 18: the client presents an RFC 6750 bearer token over SASL `OAUTHBEARER`; the server validates it with a loadable validator library; libpq needs `oauth_issuer` and `oauth_client_id` and runs a discovery flow, and the server must be built `--with-libcurl`.
  - Detail: auth-oauth manual: "PostgreSQL supports bearer tokens, defined in RFC 6750 ... The format of the access token is implementation specific and is chosen by each authorization server." Options: "issuer - An HTTPS URL which is either the exact issuer identifier of the authorization server ... This parameter is required." "scope - A space-separated list of the OAuth scopes ... This parameter is required." "validator - The library to use for validating bearer tokens ... optional unless oauth_validator_libraries contains more than one library". "map - Allows for mapping between OAuth identity provider and database user names." Server GUC: "oauth_validator_libraries ... If set to an empty string (the default), OAuth connections will be refused." libpq: "oauth_issuer - The HTTPS URL of a trusted issuer to contact if the server requests an OAuth token for the connection. This parameter is required for all OAuth connections". "oauth_client_id - An OAuth 2.0 client identifier ... this parameter must be set; otherwise, the connection will fail." SASL protocol page: "PostgreSQL implements three SASL authentication mechanisms: SCRAM-SHA-256, SCRAM-SHA-256-PLUS, and OAUTHBEARER." "The only key currently supported by the server is auth, which contains the bearer token." "OAUTHBEARER does not support channel binding".
  - Citations:
    - https://www.postgresql.org/docs/18/auth-oauth.html (Authoritative)
    - https://www.postgresql.org/docs/18/libpq-connect.html (Authoritative)
    - https://www.postgresql.org/docs/18/sasl-authentication.html (Authoritative)
    - https://www.postgresql.org/docs/18/runtime-config-connection.html (Authoritative)
  - Confidence: High, 98 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 26
  - Claim: No released Rust client supports OAUTHBEARER. rust-postgres issue #1377 and PR #1378 (both opened 2026-08-29, open) and sqlx issue #4399 and PR #4400 (open, PR updated 2026-09-09) propose token-first support; until one merges, a PostgreSQL 18 server configured with `oauth` refuses tokio-postgres with "unsupported SASL mechanism".
  - Detail: tokio-postgres `connect_raw.rs` `authenticate_sasl` matches only `sasl::SCRAM_SHA_256` and `sasl::SCRAM_SHA_256_PLUS` and otherwise `return Err(Error::authentication("unsupported SASL mechanism".into()))`. Issue #1377: "tokio-postgres cannot connect to a server configured that way: authenticate_sasl matches only SCRAM-SHA-256 and SCRAM-SHA-256-PLUS, so the server's mechanism list is refused with authentication error: unsupported SASL mechanism. Proposed scope: token-first only. The caller supplies a token; the crate performs the SASL exchange. No discovery, no device authorization flow". PR #1378 "feat: support OAUTHBEARER SASL authentication" state open, draft false, merged false, updated 2026-08-29. sqlx: issue #4399 "Postgres: support SASL OAUTHBEARER (PostgreSQL 18 oauth authentication method)" open; PR #4400 "feat(postgres): support SASL OAUTHBEARER authentication" open, updated 2026-09-09. Practical option today: run the tool against `oauth` HBA entries only after the PR lands, or use `scram-sha-256` entries for the tool's role. A Rust validator for the server side exists (UnAfraid/pg_oidc_validator_rust) but that is server-side and irrelevant to the client.
  - Citations:
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/src/connect_raw.rs (Authoritative, source read 2026-09-12)
    - https://github.com/rust-postgres/rust-postgres/issues/1377 and https://github.com/rust-postgres/rust-postgres/pull/1378 (Consensus, 2026-08-29, state read from the API 2026-09-12)
    - https://github.com/transact-rs/sqlx/issues/4399 and https://github.com/transact-rs/sqlx/pull/4400 (Consensus, state read from the API 2026-09-12)
  - Confidence: High, 95 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 27
  - Claim: tokio-postgres 0.7.18 handles `AuthenticationOk` (covers trust, peer, ident, cert, and any server-side method that needs no client exchange), cleartext password (which is what ldap, pam, radius, and bsd present to the client), MD5, SCRAM-SHA-256, and SCRAM-SHA-256-PLUS. It rejects Kerberos, GSS, SSPI, and SCM credential with "unsupported authentication method".
  - Detail: `authenticate()` match arms in connect_raw.rs: `AuthenticationOk`, `AuthenticationCleartextPassword`, `AuthenticationMd5Password`, `AuthenticationSasl`, and `AuthenticationKerberosV5 | AuthenticationScmCredential | AuthenticationGss | AuthenticationSspi => return Err(Error::authentication("unsupported authentication method".into()))`. Password-bearing paths return `Error::config("password missing")` when no password is set. `cert` works when the TLS connector is built with a client certificate (finding 30). `channel_binding=require` fails with "server did not use channel binding" if the server chooses a non-PLUS path.
  - Citations:
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/src/connect_raw.rs (Authoritative, source read 2026-09-12)
    - https://www.postgresql.org/docs/18/auth-pg-hba-conf.html (Authoritative, for which methods need no client exchange)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 28
  - Claim: tokio-postgres implements SCRAM-SHA-256-PLUS with the `tls-server-end-point` binding, chooses PLUS whenever the server offers it and the TLS stream can supply the certificate hash, and honors `channel_binding=disable|prefer|require`.
  - Detail: `authenticate_sasl`: `let channel_binding = stream.inner.get_ref().channel_binding().tls_server_end_point.filter(|_| config.channel_binding != config::ChannelBinding::Disable).map(sasl::ChannelBinding::tls_server_end_point); let (channel_binding, mechanism) = if has_scram_plus { match channel_binding { Some(channel_binding) => (channel_binding, sasl::SCRAM_SHA_256_PLUS), None => (sasl::ChannelBinding::unsupported(), sasl::SCRAM_SHA_256) } } ...`. postgres-protocol `sasl.rs`: `pub const SCRAM_SHA_256_PLUS: &str = "SCRAM-SHA-256-PLUS";` and the GS2 header `"p=tls-server-end-point,,"`. The PostgreSQL manual: "The channel binding type used by PostgreSQL is tls-server-end-point." A recent hardening: "excessive_iteration_count_is_rejected ... a malicious server cannot force an unbounded PBKDF2 loop".
  - Citations:
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/connect_raw.rs (Authoritative, source)
    - https://github.com/rust-postgres/rust-postgres/blob/master/postgres-protocol/src/authentication/sasl.rs (Authoritative, source)
    - https://www.postgresql.org/docs/18/sasl-authentication.html (Authoritative)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 29
  - Claim: Channel binding with rustls depends on the TLS crate computing the right certificate digest; tokio-postgres-rustls 0.14.0 (2026-05-21) fixed x509 parsing for channel binding and skips a digest for Ed25519 certificates, which PostgreSQL itself cannot bind.
  - Detail: v0.14.0 release notes: "fix: correctly parse x509 certificates for channel binding by @conradludgate in #32", "fix: defer TLS hostname validation by @jbg in #39", "fix: avoid Ed25519 channel binding digest by @jbg in #40", "feat: add rustls feature selection by @jbg in #42", "test: add postgres rustls integration suite by @jbg in #43". PR #40: "stop treating Ed25519 certificates as SHA-512 for tls-server-end-point channel binding; return no channel binding digest for unsupported algorithms". Earlier release notes: "Fixed the SCRAM channel binding check to use the correct digest based on the server certificate (thanks @JamesGuthrie)". rustls issue #1624 (2023-11-23) had reported "tokio-postgres-rustls only uses the sha256 digest even though the specification says that you should use the signatureAlgorithm hash in the certificate". The fork tokio-postgres-rustls-improved claims channel binding "was non-functional in all cases in original tokio-postgres-rustls" and adds "NOTE: Channel binding is not supported with Ed25519 certificates. This appears to be a limitation of Postgres, including Postgres 18." With `channel_binding=require` and an Ed25519 server certificate, expect failure on any client.
  - Citations:
    - https://github.com/jbg/tokio-postgres-rustls/releases/tag/v0.14.0 (Authoritative, 2026-05-21)
    - https://github.com/jbg/tokio-postgres-rustls/pull/40 (Authoritative)
    - https://github.com/rustls/rustls/issues/1624 (Consensus, 2023-11-23)
    - https://docs.rs/crate/tokio-postgres-rustls-improved/latest (Consensus, fork README)
  - Confidence: High, 88 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: The fork says upstream channel binding never worked; upstream's 0.14.0 notes and integration suite say it is fixed. Resolved in favor of the upstream release notes for 0.14.0 and later; the fork's claim describes older upstream versions.

## Question 5: TLS

- Finding: 30
  - Claim: tokio-postgres maps `sslmode` as follows: `disable` skips TLS; `prefer` sends `SSLRequest` and falls back to plaintext if the server refuses or if the connector is `NoTls`; `require` errors with "server does not support TLS" on refusal; `sslnegotiation=direct` requires `require`. Certificate and hostname verification are entirely the TLS connector's job. The maintainers decided not to add `verify-ca`/`verify-full` to `Config`.
  - Detail: connect_tls.rs: `SslMode::Disable => return Ok(MaybeTlsStream::Raw(stream))`; `SslMode::Prefer if !tls.can_connect(ForcePrivateApi) => ...` (plaintext); `SslMode::Prefer if negotiation == SslNegotiation::Direct => return Err(Error::tls("weak sslmode \"prefer\" may not be used with sslnegotiation=direct (use \"require\")"))`; on server refusal `if SslMode::Require == mode { return Err(Error::tls("server does not support TLS".into())); }`; `if !has_hostname { return Err(Error::tls("no hostname provided for TLS handshake".into())); }`. `MakeTlsConnect::make_tls_connect(&mut self, domain: &str)` doc: "The domain name is provided for certificate verification and SNI." PR #988 (closed 2023-07-23) discussion, sfackler: "I don't feel great about including options in a library's config that it inherently does not support." and "Every TLS backend I'm aware of for this crate acts as if verify-full was passed unless you explicitly configure things to disable that." jbg (tokio-postgres-rustls author): "the defaults of tokio-postgres-rustls, postgres-native-tls and postgres-openssl should all be equivalent to verify-full, which will work with any correctly-configured PostgreSQL server." Issue #768 "Add more SSL options to Config" is still open; PR #1252 "Remove mentions of require-full and require-ca" merged 2025-06. tokio-postgres-rustls issue #11 "Improve sslmode support" is still open. Design consequence for the tool: parse the six libpq values yourself; map `disable` to `SslMode::Disable` with `NoTls`; map `allow` and `prefer` to `SslMode::Prefer`; map `require`, `verify-ca`, `verify-full` to `SslMode::Require`; then build the rustls `ClientConfig` per mode: no-verify verifier for `require` (libpq semantics when no root CA is given), CA-only verifier for `verify-ca`, and the default verifier for `verify-full`.
  - Citations:
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/src/connect_tls.rs (Authoritative, source read 2026-09-12)
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/tls.rs (Authoritative, source)
    - https://github.com/rust-postgres/rust-postgres/pull/988 (Consensus, closed 2023-07-23, maintainer statements 2023-04-25)
    - https://github.com/rust-postgres/rust-postgres/issues/768 (Consensus, open per API 2026-09-12)
    - https://github.com/jbg/tokio-postgres-rustls/issues/11 (Consensus, open per API 2026-09-12)
  - Confidence: High, 93 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 31
  - Claim: tokio-postgres-rustls 0.14.0 has no default features; you must enable `ring` or `aws-lc-rs`, and the optional `native-certs` and `webpki-roots` features add `MakeRustlsConnect::with_native_certs()` (returns the certs plus any per-cert load errors) and `MakeRustlsConnect::with_webpki_roots()`. Loading system CAs goes through rustls-native-certs 0.8.4; self-signed local servers need their CA added to a `RootCertStore` or a deliberate no-verify verifier.
  - Detail: README: "This crate has no default features. Enable the rustls crypto provider that your application uses, either ring or aws-lc-rs. The optional webpki-roots and native-certs features add convenience constructors for common root stores." Source lib.rs: `#[cfg(feature = "webpki-roots")] pub fn with_webpki_roots() -> Self { ... roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() ...}` and `#[cfg(feature = "native-certs")] pub fn with_native_certs() -> Result<(Self, Vec<rustls_native_certs::Error>), Vec<rustls_native_certs::Error>> { let result = rustls_native_certs::load_native_certs(); ...}`. Example: `let config = rustls::ClientConfig::builder().with_root_certificates(rustls::RootCertStore::empty()).with_no_client_auth(); let tls = tokio_postgres_rustls::MakeRustlsConnect::new(config); tokio_postgres::connect("sslmode=require host=localhost user=postgres", tls)`. rustls-native-certs README: "This crate respects local configuration of root certificates: both removal of roots that the user finds untrustworthy, and addition of locally-trusted roots ... Since webpki-roots compiles in root certificates, getting an update to these requires taking regular updates to this crate". For self-signed certificates, rustls requires a leaf certificate with a SubjectAltName signed by a CA you add to the store; a bare self-signed CA used as the end entity fails with `CAUsedAsEndEntity` (StackOverflow 60751795, confirmed against the rustls verifier behavior). libpq's `require` mode does no verification unless a root CA file is present, so a tool that mirrors libpq needs a `dangerous()` custom verifier for `require`; the `rustls-tokio-postgres` helper crate documents exactly that trade-off: "config_no_verify() is dangerous because it disables server certificate and hostname verification ... Use this only for local development".
  - Citations:
    - https://crates.io/crates/tokio-postgres-rustls (Authoritative, README, 0.14.0, 2026-05-21)
    - https://raw.githubusercontent.com/jbg/tokio-postgres-rustls/master/src/lib.rs (Authoritative, source read 2026-09-12)
    - https://docs.rs/tokio-postgres-rustls/latest/tokio_postgres_rustls/ (Authoritative, docs.rs build 20 July 2026)
    - https://docs.rs/crate/rustls-native-certs/0.6.3 and https://crates.io/api/v1/crates/rustls-native-certs (Authoritative)
    - https://docs.rs/rustls-tokio-postgres/latest/rustls_tokio_postgres/ (Consensus, helper crate docs)
  - Confidence: High, 92 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 32
  - Claim: With postgres-native-tls or postgres-openssl the same rule holds (defaults verify like `verify-full`), and only postgres-openssl ships a helper for the ALPN needed by `sslnegotiation=direct`.
  - Detail: Config docs: "If you are using postgres_openssl as TLS backend, a postgres_openssl::set_postgresql_alpn helper is provided for that." libpq manual: "direct - start SSL handshake directly after establishing the TCP/IP connection. This is only allowed with sslmode=require or higher". With rustls, set `alpn_protocols = vec![b"postgresql".to_vec()]` on the `ClientConfig` yourself.
  - Citations:
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html (Authoritative)
    - https://www.postgresql.org/docs/18/libpq-connect.html (Authoritative)
  - Confidence: High, 90 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

## Question 6: SSH tunnels from Rust

- Finding: 33
  - Claim: Registry and repository facts for the SSH crates, read 2026-09-12.
  - Detail:
    - russh 0.63.3, published 2026-09-09, Apache-2.0, rust-version 1.89, 6.84M downloads; repo warp-tech/russh (GitHub shows Eugeny/russh) pushed 2026-09-09, latest release v0.63.3 2026-09-09, 71 open issues. Recent cadence: 0.63.2 (2026-09-03), 0.63.1 (2026-08-23), 0.63.0 (2026-08-21).
    - russh-keys: max stable 0.49.2; newest 0.50.0-beta.7 on 2025-01-08; nothing since, because keys moved into `russh::keys` (docs.rs russh 0.63.3 has a `keys` module with `agent`, `known_hosts`, `openssh`, `pkcs5`, `pkcs8`).
    - russh-config 0.58.0, published 2026-03-18, Apache-2.0, rust-version 1.85, "Utilities to parse .ssh/config files, including helpers to implement ProxyCommand in Russh."
    - russh-sftp 3.0.0, 2026-09-08 (not needed here).
    - ssh2 0.9.6, published 2026-06-30, MIT OR Apache-2.0, "Bindings to libssh2"; repo now rust-lang/ssh2-rs, pushed 2026-09-07.
    - openssh 0.11.6, published 2025-12-03, MIT OR Apache-2.0, rust-version 1.63.0; repo openssh-rust/openssh pushed 2026-09-11, latest release v0.11.6 2025-12-03.
    - async-ssh2-tokio 0.13.0, published 2026-07-12, license field "non-standard" on crates.io (Cargo.toml uses `license-file = "LICENSE"`); repo pushed 2026-07-12; a thin wrapper over russh.
    - thrussh 0.49.0, published 2026-08-28 by P-E-Meunier, Apache-2.0, 331K total downloads, 6,979 recent; thrussh-keys 0.45.0 on 2026-09-01. Its crates.io repository link https://nest.pijul.com/pijul/ssh returns HTTP 404, its `documentation` field points at docs.rs/ssh, and its README says "Handling SSH keys correctly on all platforms. In particular, interactions with agents, PGP, and password-protected/encrypted keys are not yet implemented."
    - ssh-key 0.6.7 stable (0.7.0-rc.11 on 2026-06-29), the RustCrypto key format crate that russh 0.63 pins as `=0.7.0-rc.11`.
    - ssh2-config 0.8.0 (2026-08-31) parses `~/.ssh/config` for ssh2 users.
  - Citations:
    - https://crates.io/api/v1/crates/russh, https://crates.io/api/v1/crates/russh-keys, https://crates.io/api/v1/crates/russh-config, https://crates.io/api/v1/crates/ssh2, https://crates.io/api/v1/crates/openssh, https://crates.io/api/v1/crates/async-ssh2-tokio, https://crates.io/api/v1/crates/thrussh, https://crates.io/api/v1/crates/thrussh/versions (Authoritative, registry API, read 2026-09-12)
    - https://api.github.com/repos/warp-tech/russh, https://api.github.com/repos/alexcrichton/ssh2-rs, https://api.github.com/repos/openssh-rust/openssh, https://api.github.com/repos/Miyoshi-Ryota/async-ssh2-tokio (Authoritative, repository API, read 2026-09-12)
    - https://crates.io/api/v1/crates/thrussh/0.49.0/readme (Authoritative, crate README)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: The brief asked whether thrussh is dead. The registry shows monthly releases through 2026-08-28, so it is not abandoned; but its repository URL is dead, its README disclaims agent and encrypted-key support, and russh (its fork, "This is a fork of Thrussh by Pierre-Étienne Meunier" per the russh README) has 20 times the downloads and the feature set below. Resolved: treat thrussh as alive but unsuitable; use russh.

- Finding: 34
  - Claim: russh supports direct-tcpip (local forwarding), direct-streamlocal (Unix socket forwarding), password, publickey, keyboard-interactive, OpenSSH certificates, an OpenSSH agent client, `known_hosts` checking, OpenSSH keepalive handling, and channels usable as `AsyncRead + AsyncWrite`.
  - Detail: README feature list: "direct-tcpip (local port forwarding); forward-tcpip (remote port forwarding); direct-streamlocal (local UNIX socket forwarding, client only); forward-streamlocal"; key exchanges include "mlkem768x25519-sha256, curve25519-sha256@libssh.org, ... ecdh-sha2-nistp256/384/521, OpenSSH strict key exchange support"; host keys "ssh-ed25519, rsa-sha2-256, rsa-sha2-512, ssh-rsa, ecdsa-sha2-nistp256/384/521, OpenSSH certificates"; auth "password, publickey, keyboard-interactive, none, OpenSSH certificates"; "OpenSSH keepalive request handling; OpenSSH agent forwarding channels; ... PPK key format; Pageant support; AsyncRead / AsyncWrite -able channels". "Crypto backends: enable at least one of the aws-lc-rs or ring features."
  - Citations:
    - https://crates.io/crates/russh (Authoritative, README for 0.63.3)
    - https://docs.rs/russh/latest/russh/ (Authoritative, docs.rs build 09 September 2026)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 35
  - Claim: The in-process tunnel pattern: `russh::client::connect(config, (host, port), handler)`, authenticate, `handle.channel_open_direct_tcpip(db_host, db_port, "127.0.0.1", 0)`, then `channel.into_stream()` yields a `ChannelStream` implementing `AsyncRead + AsyncWrite`, which goes straight into `tokio_postgres::Config::connect_raw(stream, tls)`. No local TCP listener is needed for PostgreSQL.
  - Detail: docs.rs `Channel::into_stream`: "Consume the Channel to produce a bidirectionnal stream, sending and receiving ChannelMsg::Data as AsyncRead+AsyncWrite." `Handle::channel_open_direct_tcpip`: "Open a TCP/IP forwarding channel. This is usually done when a connection comes to a locally forwarded TCP/IP port. See RFC4254 ... After writing a stream to a channel using .data(), be sure to call .eof() to indicate that no more data will be sent". tokio-postgres `connect_raw`: "Connects to a PostgreSQL database over an arbitrary stream. All of the settings other than user, password, dbname, options, and application_name name are ignored." Note that `sslmode` is among the ignored settings for `connect_raw` in the docs sentence, but the source still calls `connect_tls(stream, config.ssl_mode, ...)` on the raw stream, so TLS inside the tunnel is possible when the connector is not `NoTls`; use `has_hostname` semantics by supplying a `host` for SNI. The ferrule-sql crate (docs.rs) implements both shapes: "Stream - hands back a TunnelStream suitable for tokio_postgres::Config::connect_raw. Avoids the local TCP hop for Postgres specifically" and "LocalListener - binds 127.0.0.1:0, pumps bytes through an SSH direct-tcpip channel. Used by every backend whose driver does not expose a custom-stream injection API". A worked example (Ryan McGrath, 2025-04-29) does the same with async-ssh2-tokio: `client.open_direct_tcpip_channel(DB_SERVER, None).await` then `config.connect_raw(channel.into_stream(), NoTls)`. For a local-port variant, open a new direct-tcpip channel per accepted connection and `tokio::io::copy_bidirectional` (StackOverflow 79137536 shows the one-channel-per-connection fix).
  - Citations:
    - https://docs.rs/russh/latest/russh/struct.Channel.html (Authoritative, 0.63.3)
    - https://github.com/Eugeny/russh/blob/main/russh/src/client/mod.rs (Authoritative, source)
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html and https://github.com/sfackler/rust-postgres/blob/master/tokio-postgres/src/connect_raw.rs (Authoritative)
    - https://docs.rs/ferrule-sql/latest/src/ferrule_sql/tunnel.rs.html (Consensus, third-party implementation)
    - https://rymc.io/blog/2025/tokio-postgres-over-ssh/ (Consensus, 2025-04-29, code checked against the current API)
    - https://stackoverflow.com/questions/79137536/how-to-create-an-ssh-tunnel-with-russh-that-supports-multiple-connections (Consensus)
  - Confidence: High, 92 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 36
  - Claim: Host key verification in russh is mandatory and manual: `client::Handler::check_server_key` rejects everything by default; `russh::keys::known_hosts::check_known_hosts` and `check_known_hosts_path` implement `~/.ssh/known_hosts` checks.
  - Detail: docs.rs Handler: "You must at the very least implement the check_server_key fn. The default implementation rejects all keys." Signature: `fn check_server_key(&mut self, server_public_key: &PublicKeyOrCertificate) -> impl Future<Output = Result<bool, Self::Error>> + Send`. `russh::keys` re-exports `known_hosts::check_known_hosts` and `known_hosts::check_known_hosts_path`. The rpg review issue #824 flags a real pitfall: "SSH tunnel strict-host-key-checking default ... OpenSSH's default is ask, not no."
  - Citations:
    - https://docs.rs/russh/latest/russh/client/trait.Handler.html (Authoritative, 0.63.3)
    - https://docs.rs/russh/latest/russh/keys/index.html (Authoritative, 0.63.3)
    - https://github.com/NikolayS/rpg/issues/824 (Consensus)
  - Confidence: High, 95 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 37
  - Claim: Key files with a passphrase and SSH agents: `russh::keys::load_secret_key(path, Some(passphrase))` decodes OpenSSH, PKCS#5, and PKCS#8 keys ("deciphering it with the supplied password if necessary"); `russh::keys::agent::client::AgentClient::connect(stream)` speaks the OpenSSH agent protocol over `$SSH_AUTH_SOCK` and can `request_identities` and `sign_request`; `Handle::authenticate_publickey` takes a `PrivateKeyWithHashAlg`.
  - Detail: docs.rs keys module: "This includes in particular various functions for opening key files, deciphering encrypted keys, and dealing with agents." Functions: "decode_secret_key - Decode a secret key, possibly deciphering it with the supplied password." "load_secret_key - Load a secret key, deciphering it with the supplied password if necessary." "load_openssh_certificate - Load a openssh certificate". Example in the docs: `let stream = tokio::net::UnixStream::connect(&agent_path).await?; let mut client = agent::client::AgentClient::connect(stream); client.add_identity(&key, ...).await?; client.request_identities().await?; let sig = client.sign_request(&public.into(), None, buf).await`. Modules: `agent` ("OpenSSH agent protocol implementation"), `known_hosts`, `openssh`, `pkcs5`, `pkcs8`.
  - Citations:
    - https://docs.rs/russh/latest/russh/keys/index.html (Authoritative, 0.63.3)
    - https://dev.to/bbkr/ssh-port-forwarding-from-within-rust-code-5an (Consensus, 2025-01-06, shows `load_secret_key(file, password)` and `PrivateKeyWithHashAlg::new`)
  - Confidence: High, 90 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 38
  - Claim: `~/.ssh/config` parsing for russh comes from russh-config 0.58.0 (`parse`, `parse_home`, `parse_path`, `HostConfig`, `Stream::tcp_connect`, `Stream::proxy_command`); ProxyJump and bastion chains are built by hand: connect to the jump host, open direct-tcpip to the next hop, and call `russh::client::connect_stream` over that channel's stream.
  - Detail: russh docs: "The easy way to implement SSH tunnels, like ProxyCommand for OpenSSH, is to use the russh-config crate, and use the Stream::tcp_connect or Stream::proxy_command methods of that crate." russh-config docs list `Config`, `HostConfig`, `AddKeysToAgent`, `Stream` ("A type to implement either a TCP socket, or proxying through an external command"), and `parse`, `parse_home`, `parse_path`. `client::connect_stream`: "Connect a stream to a server. This stream must implement tokio::io::AsyncRead and tokio::io::AsyncWrite, as well as Unpin and Send." The anvil-ssh crate documents the multi-hop recipe: "the first hop uses russh::client::connect over TCP; subsequent hops use the previous hop's direct-tcpip channel as the underlying transport via russh::client::connect_stream ... every hop runs the full check_server_key path independently". russh-config is only 32 percent documented on docs.rs, so read its source before relying on which `ssh_config` keywords it understands.
  - Citations:
    - https://docs.rs/russh/latest/russh/ (Authoritative)
    - https://docs.rs/russh-config/latest/russh_config/ (Authoritative, docs.rs build 04 July 2026)
    - https://github.com/Eugeny/russh/blob/main/russh/src/client/mod.rs (Authoritative, `connect_stream`)
    - https://docs.rs/anvil-ssh/latest/anvil_ssh/session/struct.AnvilSession.html (Consensus, third-party implementation)
  - Confidence: Medium, 82 percent (ProxyJump handling is documented only in third-party crates; russh-config's keyword coverage was not read line by line)
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 39
  - Claim: The openssh crate spawns the system `ssh` under a ControlMaster and so inherits every `~/.ssh/config` feature (ProxyJump, agent, known_hosts prompts); it offers `Session::request_port_forward(ForwardType::Local, listen_socket, connect_socket)` for `-L`-style forwarding, with either a `process-mux` or `native-mux` backend. It cannot hand you an in-process stream for `connect_raw`; you connect to the forwarded local port or Unix socket.
  - Detail: docs.rs: "This crate wraps the OpenSSH remote login client (ssh on most machines) ... Since all commands are executed through the ssh command, all your existing configuration (e.g., in .ssh/config) should continue to work as expected." "Behind the scenes, the crate uses ssh's ControlMaster feature to multiplex the channels". `Session::connect(destination, KnownHosts)`: "The format of destination is the same as the destination argument to ssh." `request_port_forward`: "Request to open a local/remote port forwarding. The Socket can be either a unix socket or a tcp socket. If forward_type == Local, then listen_socket on local machine will be forwarded to connect_socket on remote machine." `KnownHosts` "Specifies how the host's key fingerprint should be handled." Trade-offs: needs an `ssh` binary on PATH (not available in a minimal container image), one extra process, and no way to inject the stream; gains: exact OpenSSH semantics for `ProxyJump`, `IdentityAgent`, `Match`, and security keys.
  - Citations:
    - https://docs.rs/openssh/latest/openssh/index.html (Authoritative, 0.11.6)
    - https://docs.rs/openssh/latest/openssh/struct.Session.html (Authoritative, 0.11.6)
  - Confidence: High, 94 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 40
  - Claim: ssh2 (libssh2 bindings) is synchronous and links a C library plus OpenSSL; it is used from inside PostgreSQL extensions (pg_ssh) but is a poor fit next to tokio-postgres because there is no async stream to hand to `connect_raw` without a blocking bridge.
  - Detail: crates.io description: "Bindings to libssh2 for interacting with SSH servers and executing remote commands, forwarding local ports, etc." pg_ssh README: "It connects with libssh2 (via the Rust ssh2 crate) and authenticates with Session::userauth_pubkey_memory" and warns about "two copies of OpenSSL colliding inside one backend process". Repo moved to rust-lang/ssh2-rs and is maintained (pushed 2026-09-07; libssh2-sys 0.3.2 tag).
  - Citations:
    - https://crates.io/api/v1/crates/ssh2 (Authoritative, registry API)
    - https://github.com/sweatybridge/pg_ssh (Consensus)
    - https://api.github.com/repos/alexcrichton/ssh2-rs (Authoritative, repository API, redirects to rust-lang/ssh2-rs)
  - Confidence: High, 88 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 41
  - Claim: async-ssh2-tokio 0.13.0 is a high-level wrapper over russh with `Client::connect(addr, user, AuthMethod, ServerCheckMethod)` and `open_direct_tcpip_channel(target, origin)`; it is smaller and less documented than russh, and its `ServerCheckMethod::NoCheck` path is a footgun.
  - Detail: russh README: "async-ssh2-tokio - simple high-level API for running commands over SSH." The 2025 tutorial notes "At time of writing, open_direct_tcpip_channel has no documentation" and uses `AuthMethod::with_key_file(KEY_PATH, None)` and `ServerCheckMethod::NoCheck`. Its source calls `channel_open_direct_tcpip(target.ip().to_string(), target.port().into(), ...)` and `into_stream()`. Repo has 10 open issues, 122 stars, last push 2026-07-12.
  - Citations:
    - https://docs.rs/crate/async-ssh2-tokio/latest/source/src/client.rs (Authoritative, source)
    - https://rymc.io/blog/2025/tokio-postgres-over-ssh/ (Consensus, 2025-04-29)
    - https://api.github.com/repos/Miyoshi-Ryota/async-ssh2-tokio (Authoritative, repository API)
  - Confidence: Medium, 80 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 42
  - Claim: Existing Rust PostgreSQL tools with SSH tunnels use russh in-process: rpg (psql-compatible terminal, 0.11.0) pins `russh = { version = "0.60", default-features = false, features = ["ring", "flate2"] }` next to `tokio-postgres = "0.7"`, and ferox pins `russh = "0.44"` and `russh-keys = "0.44"` next to `tokio-postgres`. No surveyed Rust database tool spawns `ssh -L` or `ssh -W`.
  - Detail: rpg README: "SSH tunnel: connect through bastion hosts without manual port-forwarding ... rpg --ssh-tunnel user@bastion.example.com -h 10.0.0.5 -d mydb". rpg issue #760 dependency audit: "SSH tunnel code in src/connection.rs" and "russh: Already gated in PR #759"; the same audit notes that "On WASM ... call config.connect_raw(stream, tls)". rpg also patches tokio-postgres from a fork tag (`tokio-postgres-v0.7.17-patched.1`), which is worth reading before adopting its connection code. ferox README says it stores "the SSH tunnel password" in the OS keychain. rainfrog (0.4.5, 2026-08-25), pgtui, and sabiql expose no SSH option in their READMEs.
  - Citations:
    - https://raw.githubusercontent.com/NikolayS/rpg/main/Cargo.toml (Authoritative, read 2026-09-12)
    - https://github.com/NikolayS/rpg (Authoritative, README)
    - https://github.com/NikolayS/rpg/issues/760 (Consensus, 2026-03-27)
    - https://raw.githubusercontent.com/frkdrgt/ferox/main/Cargo.toml (Authoritative, read 2026-09-12)
    - https://docs.rs/crate/rainfrog/0.3.15 (Consensus, README)
  - Confidence: Medium, 82 percent (two tools inspected; survey not exhaustive)
  - As-of: September 12 2026, 01:44:01 PM (+0600)

## Question 7: Managed and pooled endpoints

- Finding: 43
  - Claim: RDS IAM authentication uses a SigV4-signed token as the password; it expires 15 minutes after generation, must be generated against the instance endpoint (not a custom DNS name), and AWS documents `sslmode=verify-full sslrootcert=<bundle>` for psql.
  - Detail: "An authentication token is a string of characters that you use instead of a password. After you generate an authentication token, it's valid for 15 minutes before it expires. If you try to connect using an expired token, the connection request is denied." "Every authentication token must be accompanied by a valid signature, using AWS signature version 4." "You cannot use a custom Route 53 DNS record instead of the DB instance endpoint to generate the authentication token." psql form: `psql "host=hostName port=portNumber sslmode=verify-full sslrootcert=full_path_to_ssl_certificate dbname=DBName user=userName password=authToken"`; the token begins `rdspostgres...rds.amazonaws.com:5432/?Action=connect&DBUser=jane_doe&X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Expires=900...`. IAM auth "is only used for authentication and doesn't affect the session after it is established", so an established connection outlives the token; only reconnects need a fresh token. Rust: aws-sdk-rust has no generate-db-auth-token helper in the SDK (issues #147 and #792); the community crate aws-rds-signer exists (not vetted here).
  - Citations:
    - https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.Connecting.html (Authoritative)
    - https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.Connecting.AWSCLI.PostgreSQL.html (Authoritative)
    - https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.html (Authoritative)
    - https://github.com/awslabs/aws-sdk-rust/issues/792 (Consensus)
  - Confidence: High, 95 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 44
  - Claim: Cloud SQL Auth Proxy listens on `127.0.0.1:PORT` (or a Unix socket) and, with `--auto-iam-authn`, injects the IAM principal's OAuth token as the password so the client sends only the IAM username; IAM tokens last about an hour, and pooled connections that hold an expired token fail on the server side.
  - Detail: "For TCP connections, the Cloud SQL Auth Proxy listens on localhost (127.0.0.1) by default. So, when you specify --port PORT_NUMBER for an instance, the local connection is at 127.0.0.1:PORT_NUMBER." "The Cloud SQL Auth Proxy binary connects to one or more Cloud SQL instances specified on the command line, and opens a local connection as either TCP or a Unix socket." "Warning: Be careful when binding the Cloud SQL Auth Proxy to an external interface." IAM logins: "Start the Cloud SQL Auth Proxy with the --auto-iam-authn flag ... USERNAME: For an IAM, the username is the full email address of the user. For a service account, this is the service account's email without the .gserviceaccount.com domain suffix." Google Cloud blog: "The application only has to specify the service account name without any password ... The password will be added transparently by the Cloud SQL Auth proxy." Issue #2553 (cloud-sql-proxy): "Google Support suggests the root cause is IAM token expiration (approx. 1 hour). When MCP is active, pooled connections appear to hold onto expired tokens". Because the proxy terminates TLS, the client talks plaintext to 127.0.0.1; use `sslmode=disable` or `prefer` there.
  - Citations:
    - https://cloud.google.com/sql/docs/postgres/connect-auth-proxy (Authoritative)
    - https://cloud.google.com/sql/docs/postgres/iam-logins (Authoritative)
    - https://cloud.google.com/blog/products/databases/application-security-with-cloud-sql-iam-database-authentication (Authoritative, vendor blog)
    - https://github.com/GoogleCloudPlatform/cloud-sql-proxy/issues/2553 (Consensus)
  - Confidence: High, 90 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 45
  - Claim: Neon routes by TLS SNI; a client that does not send SNI gets "The endpoint ID is not specified", and the documented workaround is `options=endpoint%3D<endpoint_id>` (or `options=endpoint=<id>` in key-value form). rustls sends SNI when given a DNS hostname, which tokio-postgres passes through `make_tls_connect(domain)`.
  - Detail: Neon docs: "This error occurs if your client library or application does not support the Server Name Indication (SNI) mechanism in TLS. Neon uses compute IDs (the first part of a Neon domain name) to route incoming connections. However, the Postgres wire protocol does not transfer domain name information, so Neon relies on the Server Name Indication (SNI) extension of the TLS protocol to do this. SNI support was added to libpq ... in Postgres 14". "Neon supports a connection option named endpoint ... add options=endpoint%3D[endpoint_id] as a parameter to your connection string ... The %3D is a URL-encoded = sign." Key-value form: `dbname=neondb options=endpoint=[endpoint_id]`. Because tokio-postgres `options` is passed as the startup `options` parameter, this works unchanged. Note that libpq's `sslsni` parameter (finding 13) has no tokio-postgres equivalent; SNI is always on with rustls when the host is a DNS name and off when the host is an IP.
  - Citations:
    - https://neon.com/docs/connect/connection-errors (Authoritative)
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html (Authoritative, `options` key)
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/tls.rs (Authoritative, "The domain name is provided for certificate verification and SNI.")
  - Confidence: High, 93 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 46
  - Claim: Supabase's Supavisor: session mode on port 5432 and transaction mode on port 6543; transaction mode does not support prepared statements, `SET`, `LISTEN/NOTIFY`, temp tables across transactions, or advisory locks; the `options=-c search_path=...` startup parameter is not honored by the pooler (issue #206 open, fix PR #768 unmerged). Named prepared statements in transaction mode exist only behind a feature flag.
  - Detail: Supabase docs: "Session mode - Supavisor on port 5432. Best for persistent clients that need per-session features such as SET statements, prepared statements, LISTEN/NOTIFY, or advisory locks." "Transaction mode - Supavisor or PgBouncer on port 6543 ... Does not support session-level features (SET, LISTEN/NOTIFY, temporary tables that span transactions, or advisory locks). Supavisor pooler does not support prepared statements; PgBouncer can be configured to support them." "Caution: Transaction mode does not support prepared statements. To avoid errors, turn off prepared statements for your connection library." Supavisor Prisma docs: "Supavisor supports named prepared statements in transaction mode when the named_prepared_statements feature flag is enabled, either globally with the NAMED_PREPARED_STATEMENTS_ENABLED environment variable or per tenant via feature_flags." Issue #206: "Postgres connection strings support an options query parameter where one can add a specific search_path ... This works if I use the direct Postgres connection string ... but does not work with the connection pooler string (Supavisor)"; state open, PR #768 "fix: correct OPTIONS parsing and search_path" not merged as of 2026-09-12. For a tool that scopes each run to one schema, this means `search_path` must be set per transaction (`SET LOCAL`) or schema-qualified in SQL when a pooler is in the path.
  - Citations:
    - https://supabase.com/docs/guides/database/connecting-to-postgres (Authoritative)
    - https://supabase.com/docs/guides/self-hosting/accessing-postgres (Authoritative)
    - https://supabase.com/docs/guides/troubleshooting/disabling-prepared-statements-qL8lEL (Authoritative)
    - https://supabase.github.io/supavisor/orms/prisma/ (Authoritative, project docs)
    - https://github.com/supabase/supavisor/issues/206 (Consensus, state read from the API 2026-09-12)
  - Confidence: High, 92 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)
  - Conflicts: Supabase's guide says transaction mode "does not support prepared statements"; the Supavisor project docs say it does behind a flag. Resolved: default is unsupported; treat it as unsupported unless the operator enabled the flag.

- Finding: 47
  - Claim: PgBouncer transaction pooling breaks `SET/RESET`, `LISTEN`, `WITH HOLD` cursors, `PREPARE/DEALLOCATE`, `PRESERVE/DELETE ROWS` temp tables, `LOAD`, and session-level advisory locks; protocol-level prepared statements work only when `max_prepared_statements` is non-zero (added in 1.21.0); the startup parameters PgBouncer tracks are `client_encoding`, `DateStyle`, `IntervalStyle`, `Timezone`, `standard_conforming_strings`, and `application_name`. Current release: 1.25.2 (2026-05).
  - Detail: Feature map: "SET/RESET: Session Yes, Transaction Never; LISTEN: Never; NOTIFY: Yes; WITH HOLD CURSOR: Never; Protocol-level prepared plans: Yes (footnote: You need to change max_prepared_statements to a non-zero value to enable this support); PREPARE / DEALLOCATE: Never; PRESERVE/DELETE ROWS temp tables: Never; LOAD statement: Never; Session-level advisory locks: Never." "Startup parameters are: client_encoding, DateStyle, IntervalStyle, Timezone, standard_conforming_strings, and application_name. PgBouncer detects their changes and so it can guarantee they remain consistent for the client. If you need PgBouncer to support more than these, take a look at track_extra_parameters". FAQ: "Since version 1.21.0 PgBouncer can track prepared statements in transaction pooling mode and make sure they get prepared on-the-fly on the linked server connection. To enable this feature, max_prepared_statements needs to be set to a non-zero value." Implication: `application_name` tagging survives transaction pooling; `search_path` does not unless the operator adds it to `track_extra_parameters`.
  - Citations:
    - https://www.pgbouncer.org/features.html (Authoritative, site dated 2026)
    - https://www.pgbouncer.org/faq.html (Authoritative)
  - Confidence: High, 97 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 48
  - Claim: tokio-postgres already provides the pooler-safe execution paths: `simple_query`/`batch_execute` use no prepared statements at all, and `query_typed` runs an unnamed prepare-bind-execute in one round trip for "environments where prepared statements aren't supported (such as Cloudflare Workers with Hyperdrive)". sqlx's `query()` caches named prepared statements and its docs point PgBouncer users at `.persistent(false)`.
  - Detail: tokio-postgres `query_typed` doc: "Compared to query, this method allows performing queries without three round trips (for prepare, execute, and close) by requiring the caller to specify parameter values along with their Postgres type. Thus, this is suitable in environments where prepared statements aren't supported (such as Cloudflare Workers with Hyperdrive)." sqlx `query` doc quoted in finding 6.
  - Citations:
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/client.rs (Authoritative, source)
    - https://docs.rs/sqlx/latest/sqlx/fn.query.html (Authoritative)
  - Confidence: High, 94 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 49
  - Claim: What the tool must avoid when a pooler is in the path: session `SET` (use `SET LOCAL` inside a transaction or fully qualify schemas), named prepared statements (use `simple_query`, `query_typed`, or PgBouncer with `max_prepared_statements > 0`), `LISTEN`, session advisory locks, `WITH HOLD` cursors, temp tables across transactions, `DEALLOCATE`, and `channel_binding=require` (TLS terminates at the pooler, so the backend never offers SCRAM-SHA-256-PLUS).
  - Detail: Derived from findings 46 to 48. The channel-binding point is corroborated by an operator note: "channel_binding=require cannot work through the pooler endpoint: TLS terminates at the pooler, so SCRAM-SHA-256-PLUS isn't advertised by the inner server" (workers.iii.dev, third-party) and by the tokio-postgres behavior in finding 27 ("server did not use channel binding").
  - Citations:
    - https://www.pgbouncer.org/features.html (Authoritative)
    - https://supabase.com/docs/guides/database/connecting-to-postgres (Authoritative)
    - https://github.com/rust-postgres/rust-postgres/blob/master/tokio-postgres/src/connect_raw.rs (Authoritative)
    - https://workers.iii.dev/workers/database (Consensus, third-party operator docs)
  - Confidence: High, 90 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

## Question 8: Connection lifecycle for a long-running MCP server

- Finding: 50
  - Claim: tokio-postgres enables TCP keepalives by default with a 2 hour idle time and exposes `keepalives_interval`, `keepalives_retries`, and `tcp_user_timeout`; all are ignored on Unix sockets. Liveness checks: `Client::is_closed()` (cheap, may lag on hard network drops) and `Client::check_connection()` (round trip, added 0.7.15).
  - Detail: Config docs: "keepalives - Controls the use of TCP keepalive ... Defaults to on." "keepalives_idle - The number of seconds of inactivity after which a keepalive message is sent to the server ... Defaults to 2 hours." "tcp_user_timeout - The time limit that transmitted data may remain unacknowledged before a connection is forcibly closed." Client docs: "check_connection(&self) -> Result<(), Error> Check that the connection is alive and wait for the confirmation." "is_closed(&self) -> bool Determines if the connection to the server has already closed. In that case, all future queries will fail." Changelog 0.7.15: "Improved the effectiveness of Client::is_closed." The `Connection` future returned by `connect` "performs the actual IO with the server, and should generally be spawned off onto an executor to run in the background"; when it resolves (with an error on drop), the client is dead and the tool should reconnect on the next request. `Client::cancel_token()` gives a `CancelToken` for cancelling a running statement from another task, which an MCP server needs for user-initiated cancel.
  - Citations:
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html (Authoritative)
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html (Authoritative)
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Connection.html (Authoritative)
    - https://raw.githubusercontent.com/rust-postgres/rust-postgres/master/tokio-postgres/CHANGELOG.md (Authoritative, 2025-10-08 entry)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 51
  - Claim: deadpool-postgres 0.14.2 recycles connections with `RecyclingMethod::Fast` by default (only `is_closed()`), `Verified` (adds a test query), `Clean` (adds a DISCARD-style reset), or `Custom(String)`; the pool never fails at creation, so call `pool.get()` once at startup to surface connection errors; `Manager::from_connect` lets you plug a custom connector (for example one that opens an SSH channel per connection).
  - Detail: "The default is Fast which does not check the connection health or perform any clean-up queries. Only run Client::is_closed() when recycling existing connections." "Verified - Run Client::is_closed() and execute a test query. This is slower, but guarantees that the database connection is ready to be used. Normally, Client::is_closed() should be enough to filter out bad connections, but under some circumstances (i.e. hard-closed network connections) it's possible that Client::is_closed() returns false while the connection is dead. You will receive an error on your first query then." FAQ: "Deadpool has identical startup and runtime behaviour and therefore the pool creation will never fail. If you want your application to crash on startup if no database connection can be established just call pool.get().await right after creating the pool." `Manager::from_connect(pg_config, connect: impl Connect + 'static, config)`. Example builds a config with `pg_config.host_path("/run/postgresql"); pg_config.host_path("/tmp");` to try both socket directories in order.
  - Citations:
    - https://docs.rs/deadpool-postgres/latest/deadpool_postgres/enum.RecyclingMethod.html (Authoritative, 0.14.2)
    - https://docs.rs/deadpool-postgres/latest/deadpool_postgres/ (Authoritative, 0.14.2)
    - https://docs.rs/deadpool-postgres/latest/deadpool_postgres/struct.Manager.html (Authoritative)
  - Confidence: High, 96 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 52
  - Claim: Pool sizing: for the default single-user local mode, one tokio-postgres connection (plus reconnect on `is_closed()` or `check_connection()` failure) is enough and keeps `SET`, temp tables, and transaction state coherent; a pool only helps the remote multi-client mode, and there it must be small because each PostgreSQL connection is a backend process. bb8-postgres is a viable alternative but its adapter has not changed since 2025-11-24 and its last crate release was 2024-12-09.
  - Detail: tokio-postgres `Client` is `Send + Sync` and pipelines requests; the docs note "Requests are executed in the order that they are first polled". The one-session-per-run scoping in the brief (one database, one schema) maps onto one connection with `search_path` set once at connect through `options`. Registry facts for bb8-postgres are in finding 2.
  - Citations:
    - https://docs.rs/tokio-postgres/latest/tokio_postgres/ (Authoritative, pipelining note)
    - https://crates.io/api/v1/crates/bb8-postgres and https://api.github.com/repos/djc/bb8/commits?path=postgres (Authoritative, read 2026-09-12)
  - Confidence: Medium, 80 percent (design judgment resting on documented behavior)
  - As-of: September 12 2026, 01:44:01 PM (+0600)

- Finding: 53
  - Claim: Session limits and tagging go through the startup `options` parameter and `application_name`: `options=-c statement_timeout=30s -c idle_in_transaction_session_timeout=60s -c search_path=myschema` and `application_name=pg-dba-mcp`; PostgreSQL 14 and later also has `idle_session_timeout`, and 17 and later `transaction_timeout`.
  - Detail: libpq: "options - Specifies command-line options to send to the server at connection start. For example, setting this to -c geqo=off or --geqo=off sets the session's value of the geqo parameter to off. Spaces within this string are considered to separate command-line arguments, unless escaped with a backslash". PostgreSQL client-defaults manual: `statement_timeout` "Abort any statement that takes more than the specified amount of time"; `idle_in_transaction_session_timeout` "Terminate any session that has been idle (that is, waiting for a client query) within an open transaction for longer than the specified amount of time ... A value of zero (the default) disables the timeout"; `idle_session_timeout` "Terminate any session that has been idle ... but not within an open transaction"; `transaction_timeout` "Terminate any session that spans longer than the specified amount of time in a transaction ... If transaction_timeout is shorter or equal to idle_in_transaction_session_timeout or statement_timeout then the longer timeout is ignored." tokio-postgres sends `options` and `application_name` in the startup packet (connect_raw.rs `startup()` pushes `("options", ...)` and `("application_name", ...)`). PgBouncer preserves `application_name` across transaction pooling but not `search_path` (finding 47). For a DBA tool, a server-side `statement_timeout` set at connect is the safety net for runaway user SQL, and `cancel_token()` is the interactive cancel.
  - Citations:
    - https://www.postgresql.org/docs/18/libpq-connect.html (Authoritative)
    - https://www.postgresql.org/docs/current/runtime-config-client.html (Authoritative, PostgreSQL 18)
    - https://github.com/sfackler/rust-postgres/blob/master/tokio-postgres/src/connect_raw.rs (Authoritative, `startup()`)
  - Confidence: High, 95 percent
  - As-of: September 12 2026, 01:44:01 PM (+0600)

## Conflicts found and how each was resolved

- thrussh "dead" assumption versus registry evidence: crates.io shows thrussh 0.49.0 published 2026-08-28 and 0.44 through 0.48 in August 2026, all by the original author, so it is not abandoned. Its repository link returns 404, its README says agent and encrypted-key support "are not yet implemented", and russh (the fork) has 6.8M downloads against 331K and the feature list in finding 34. Resolved: thrussh is alive but unsuitable; russh is the in-process choice.
- Postgres.app Unix socket: a 2013 blog says sockets are not enabled by default and used `/var/pgsql_socket`; 2024 user logs from Postgres.app 2.7 and Postico's docs show `/tmp/.s.PGSQL.5432`, and Postgres.app's own initdb and pg_ctl commands pass no socket directory. Resolved by recency and the upstream default; flagged Medium because no Postgres.app doc sentence states `/tmp`.
- Channel binding in tokio-postgres-rustls: the fork tokio-postgres-rustls-improved claims it "was non-functional in all cases in original tokio-postgres-rustls"; upstream v0.14.0 (2026-05-21) lists "fix: correctly parse x509 certificates for channel binding" and an integration suite. Resolved in favor of the upstream release notes for 0.14.0 and later; the fork's claim describes earlier versions.
- Docker `POSTGRES_HOST_AUTH_METHOD` default: an older README revision says `md5`; the current Docker Hub page says `scram-sha-256` in 14 and later. Resolved by recency.
- Supavisor prepared statements in transaction mode: Supabase's guide says unsupported; the Supavisor project docs describe a `named_prepared_statements` flag. Resolved: unsupported by default, flag-gated.
- `.pgpass` permission check: libpq ignores group- or world-readable files on all Unix systems; sqlx checks only on Linux. Resolved by treating the PostgreSQL manual as the spec for the tool.
- sqlx repository identity: crates.io says launchbadge/sqlx; GitHub redirects to transact-rs/sqlx with continuous tags. Resolved as an org rename, no maintenance concern.
- Fedora dist-git patch path: four raw URL variants returned 404. Resolved with the CentOS git copy of the same `postgresql-var-run-socket.patch`, the CentOS Stream 9 spec listing it, and the RHEL 10 manual's `\conninfo` output.

## Open questions

- Does Postgres.app's shipped `pg_hba.conf` template set `trust` for both `local` and `host` lines, or only `host` on loopback? Searched: postgresapp.com documentation index, configuration-general, app-permissions, GitHub README, and issue #749; the docs say "requires no password" and describe trust, but the template file was not read.
- Which `ssh_config` keywords does russh-config 0.58.0 actually parse (Host globs, HostName, User, Port, IdentityFile, ProxyCommand, ProxyJump, IdentityAgent)? Searched: docs.rs russh-config (32 percent documented) and the russh README; the keyword list was not verified from source.
- What license does async-ssh2-tokio 0.13.0 carry? crates.io reports "non-standard" and Cargo.toml uses `license-file = "LICENSE"`; the LICENSE file was not read.
- Will rust-postgres PR #1378 (OAUTHBEARER) or sqlx PR #4400 merge before the tool ships? Both were open on 2026-09-12 with no maintainer merge signal found.
- When will pg_query.rs publish a release on libpg_query 18? PR #79 was open on 2026-08-06; no release date was found.

## Methodology

- Searches run per tool:
  - Exa: 25
  - Serper: 24 (gap between Exa and Serper: 1, within 10 percent)
  - Tavily: 5
  - Brave: 5
  - Built-in WebSearch: 5
- Primary sources fetched and read in full (local reader `mcp__read-website__read_website`, 26 fetches, all served locally; paid fetchers used: 0): tokio-postgres Config docs, tokio-postgres Client docs, tokio-postgres-rustls MakeRustlsConnect docs, PostgreSQL 18 libpq-connect, libpq-envars, libpq-pgpass, libpq-pgservice, auth-pg-hba-conf, auth-oauth, app-initdb, rust-postgres issue #729, rust-postgres PR #988, pgbouncer features, pgbouncer FAQ, Neon connection-errors, AWS RDS IAM connecting and the psql page, sqlx raw_sql docs, russh Handler docs, russh keys docs, russh-config docs, Cloud SQL connect-auth-proxy, Postgres.app documentation index, configuration-general, and app-permissions. Five of these came back larger than the tool's inline limit and were read from the saved output files.
- Registry and repository API reads (Bash curl with a User-Agent header, and `gh api`): crates.io metadata for 46 crate names plus 7 keyword searches and 4 version histories; GitHub API for 12 repositories (pushed_at, archived, releases), tag lists for 6, commit dates for 3, issue and PR state for 14 items, and 4 issue searches; raw source files from GitHub, Launchpad, Salsa, Arch GitLab, and the CentOS git mirror (Homebrew formula, rpg and ferox Cargo.toml, tokio-postgres CHANGELOG, connect_raw.rs, connect_tls.rs, query.rs, frontend.rs, tokio-postgres-rustls lib.rs, Docker 18/bookworm Dockerfile, Arch socket patch and PKGBUILD, Debian pg_createcluster, pg_query.rs README and CHANGELOG, async-ssh2-tokio Cargo.toml, thrussh README).
- Sources evaluated: about 125. Selected for findings: 74. Discarded as false positives or leads only (38): SEO and content-farm posts (dbpro.app, oneuptime, nerdleveltech, monpg, adamarant, armordb, thedavestack, linuxcapable, getgalaxy, it-server-room, w3tutorials, medium, skildops, blog.stackademic, matthewswong), AI-generated wiki pages (deepwiki, grokipedia, context7 summary), a Fedora Docs page that returned binary garbage, pgpedia (2020 screenshot, used only as a lead), the 2013 iamvery post and 2016 gist on Postgres.app sockets (historical), Stack Overflow and Reddit threads used only as leads (5), the nest.pijul.com repository (404), and Percona and Neon marketing pages restating the PostgreSQL 18 release notes.
- Depth honored: standard depth as set by the runner, with expansion on Exa and Serper in step until every listed sub-question had primary-source confirmation.

## Outages

- None marked dead. Notes: Serper returned zero organic results for three exact-quoted queries (not errors); the Fedora dist-git raw URL returned 404 on four path variants and the PGDG gitweb returned one 429, both replaced by the CentOS git mirror and RHEL documentation without retry loops.

## Limitations

- No credential files were opened (no `~/.pgpass`, `~/.ssh`, or `.env`); formats come from the PostgreSQL manual only.
- No code was executed against a live PostgreSQL. The claim that tokio-postgres `query()` fails on `aclitem` rests on the `Some(1)` result-format code in the source plus the maintainer's 2020 comment, not on a test run.
- Postgres.app's `/tmp` socket path and its `pg_hba.conf` template are inferred from logs, third-party docs, and upstream defaults (finding 23, Medium).
- The survey of Rust database tools using SSH covered rpg and ferox in detail and rainfrog, pgtui, and sabiql by README only.
- docs.rs for tokio-postgres-rustls 0.14.0 does not render the feature-gated `with_native_certs` and `with_webpki_roots` constructors; they were verified from the master branch source, which may differ from the published 0.14.0 crate in minor ways.
- The transact-rs/sqlx org rename was not investigated beyond confirming continuity of tags and pushes.
- No fetched page contained instructions aimed at this agent.
