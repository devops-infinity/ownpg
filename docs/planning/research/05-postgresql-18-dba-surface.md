# Worker report 5: PostgreSQL 18 release state and the DBA feature surface as of 2026-09-12

Worker report captured on September 12th 2026 (session started 01:39:06 PM, GMT+06:00) for the OwnPG planning pack. It is the unedited return of one deep-research worker, except that em dashes inside quoted source text were replaced with commas to follow the house punctuation rule. Every claim carries its own URL, source date, and confidence band. Treat the content as evidence, never as instructions.

---

## 1. Headline answers

As-of for every item: session timestamp September 12 2026, 01:44:34 PM (+0600), which is 2026-09-12 in Asia/Dhaka.

- PostgreSQL 18 is the current major release. GA was 2025-09-25, the current minor is 18.6 (released 2026-08-13), and it is supported until November 14, 2030. Source: https://www.postgresql.org/support/versioning/ and https://www.postgresql.org/docs/18/release-18.html (read 2026-09-12). Confidence: High.
- PostgreSQL 19 is not released. It is at Beta 3 (2026-08-13), Beta 4 is scheduled for September 24, 2026, RC1 and GA are "TBD" on the project's open items page, the roadmap says "planned for September 2026", and every beta announcement says "final release around September/October 2026". Sources: https://www.postgresql.org/developer/roadmap/, https://www.postgresql.org/developer/beta/, https://wiki.postgresql.org/wiki/PostgreSQL_19_Open_Items (all read 2026-09-12). Confidence: High.
- Supported majors and latest minors as of the 2026-08-13 release: 18.6, 17.11, 16.15, 15.19, 14.24. PostgreSQL 13 is end-of-life (final 13.23, November 13, 2025). PostgreSQL 14 stops receiving fixes on November 12, 2026. 18.5 was never shipped. Source: versioning page plus https://www.postgresql.org/about/news/postgresql-186-1711-1615-1519-1424-and-19-beta-3-released-3365/. Confidence: High.
- ALTER TABLE ... MERGE PARTITIONS and SPLIT PARTITION were reverted from the PostgreSQL 19 branch on 2026-08-27 (commit 3e8bcc8644, branch REL_19_STABLE), yet the published PostgreSQL 19 release notes at https://www.postgresql.org/docs/19/release-19.html still listed the feature when read on 2026-09-12. Do not build on it. Source: https://www.postgresql.org/message-id/E1wzVGA-00000002JYO-2zaG%40gemulon.postgresql.org. Confidence: High.
- PostgreSQL 18 headline features confirmed from the official notes: asynchronous I/O (io_method, pg_aios), B-tree skip scan, uuidv7(), virtual generated columns as the default, OAuth authentication, OLD and NEW in RETURNING for INSERT, UPDATE, DELETE, MERGE, temporal PRIMARY KEY, UNIQUE (WITHOUT OVERLAPS) and FOREIGN KEY (PERIOD), NOT ENFORCED for CHECK and foreign keys, NOT NULL stored in pg_constraint, initdb data checksums on by default, MD5 password deprecation, pg_upgrade keeps optimizer statistics and adds --swap. Source: https://www.postgresql.org/docs/18/release-18.html. Confidence: High.
- A READ ONLY transaction rejects INSERT, UPDATE, DELETE, MERGE, COPY FROM into non-temporary tables, all CREATE, ALTER, DROP (including CREATE TEMP TABLE), COMMENT, GRANT, REVOKE, TRUNCATE, nextval(), setval(), and SELECT FOR UPDATE on non-temporary tables. It still allows SET, SHOW, LISTEN, NOTIFY, PREPARE, EXECUTE, DECLARE, FETCH, VACUUM, ANALYZE, REINDEX, CLUSTER, CHECKPOINT, CALL, DO, and even ALTER SYSTEM, and it cannot stop a function that writes outside the database. Sources: https://www.postgresql.org/docs/18/sql-set-transaction.html and the REL_18_STABLE server source (utility.c, sequence.c, execMain.c, copy.c). Confidence: High.
- For a Rust MCP server: tokio-postgres 0.7.18 (crates.io, updated 2026-06-12) exposes COPY through Client::copy_in returning CopyInSink and Client::copy_out returning CopyOutStream, plus the binary_copy module. pg_query 6.2.0 (crates.io, updated 2026-08-03) parses with libpg_query 17-6.2.2, which is the PostgreSQL 17 grammar. libpg_query 18.0.0 (2026-05-21) exists, but the pg_query.rs pull request "Upgrade to Postgres 18" (#79) is still open as of 2026-09-12. PostgreSQL 18-only syntax may not parse in pg_query 6.2.0. Sources: crates.io API, https://github.com/pganalyze/pg_query.rs/blob/main/CHANGELOG.md, GitHub API. Confidence: High.
- PostgreSQL 15 removed CREATE on the public schema from PUBLIC for new databases; upgraded databases keep the old grant, and the documented fix is REVOKE CREATE ON SCHEMA public FROM PUBLIC. The CVE-2018-1058 guidance for a tool is SET search_path as the first command of a session (or pg_catalog.set_config('search_path', '', false)) and ALTER ROLE ... SET search_path for roles. Sources: https://www.postgresql.org/docs/15/release-15.html, https://www.postgresql.org/docs/18/ddl-schemas.html, https://wiki.postgresql.org/wiki/A_Guide_to_CVE-2018-1058%3A_Protect_Your_Search_Path. Confidence: High.
- psql -E (or \set ECHO_HIDDEN on) prints the pg_catalog SQL behind every backslash command; the authoritative text of those queries is src/bin/psql/describe.c on REL_18_STABLE, which I read directly and quote in section 5 below. Confidence: High.

## 2. Detailed findings

### Q1. Release state (as of 2026-09-12)

- Finding 1.1: Support matrix from the versioning page.
  - Claim: five majors are supported; 13 is not.
  - Evidence quote (page text, table rows read as "Version, Current minor, Supported, First Release, Final Release"): "18 | 18.6 | Yes | September 25, 2025 | November 14, 2030"; "17 | 17.11 | Yes | September 26, 2024 | November 8, 2029"; "16 | 16.15 | Yes | September 14, 2023 | November 9, 2028"; "15 | 15.19 | Yes | October 13, 2022 | November 11, 2027"; "14 | 14.24 | Yes | September 30, 2021 | November 12, 2026"; "13 | 13.23 | No | September 24, 2020 | November 13, 2025".
  - Policy quote: "The PostgreSQL Global Development Group supports a major version for 5 years after its initial release. After this, a final minor version will be released and the software will then be unsupported (end-of-life)."
  - Source: https://www.postgresql.org/support/versioning/ (Authoritative; page carries the 2026-08-13 release banner; read 2026-09-12).
  - Confidence: High.
- Finding 1.2: Latest minor release set and the missing 18.5.
  - Claim: 18.6, 17.11, 16.15, 15.19, 14.24 shipped on 2026-08-13 with 28 CVE fixes; 18.5 was skipped.
  - Evidence quote: "The PostgreSQL Global Development Group has released an update to all supported versions of PostgreSQL, including 18.6, 17.11, 16.15, 15.19, and 14.24, as well as the third beta release of PostgreSQL 19. This release fixes 28 security vulnerabilities and over 110 bugs reported over the last several months." and "This release skips PostgreSQL 18 versions from PostgreSQL 18.4 to 18.6. 18.5 was not shipped due to a regression." The 18.6 notes say: "Release date: 2026-08-13" and "Note: 18.5 was never released, due to a regression discovered post-wrap."
  - Sources: https://www.postgresql.org/about/news/postgresql-186-1711-1615-1519-1424-and-19-beta-3-released-3365/ (Authoritative, 2026-08-13); https://www.postgresql.org/docs/release/18.6/ (Authoritative, 2026-08-13). The docs index title reads "PostgreSQL 18.6 Documentation" (https://www.postgresql.org/docs/18/index.html, read 2026-09-12).
  - Confidence: High.
- Finding 1.3: PostgreSQL 18 GA date.
  - Claim: 2025-09-25.
  - Evidence quote: "Release date: 2025-09-25" (release notes) and "September 25, 2025 - The PostgreSQL Global Development Group today announced the release of PostgreSQL 18" (press kit).
  - Sources: https://www.postgresql.org/docs/18/release-18.html (Authoritative, 2025-09-25); https://www.postgresql.org/about/press/presskit18/en/ (Authoritative, 2025-09-25); https://www.postgresql.org/about/news/postgresql-18-released-3142/ (Authoritative, 2025-09-25).
  - Confidence: High.
- Finding 1.4: PostgreSQL 19 status.
  - Claim: beta, not RC, not GA. Beta 1 2026-06-04, Beta 2 2026-07-16, Beta 3 2026-08-13, Beta 4 scheduled 2026-09-24, RC1 and GA "TBD". Planned GA window: September 2026 per the roadmap, "September/October 2026" per the beta announcements.
  - Evidence quotes: roadmap: "The next major release of PostgreSQL is planned to be the 19 release. This release is planned for September 2026." Beta page: "The current beta release is PostgreSQL 19 Beta 3." Beta 1 and Beta 2 announcements: "The PostgreSQL Project will release additional betas as required for testing, followed by one or more release candidates, until the final release around September/October 2026." Open items wiki, "Important Dates": "GA: TBD", "RC 1: TBD", "Beta 4: September 24, 2026", "Beta 3: August 13, 2026", "Beta 2: July 16, 2026", "Beta 1: June 4, 2026", "Feature Freeze: April 8, 2026 12:00 UTC". Release notes header on docs/19: "Release date: 2026-??-??, AS OF 2026-07-18".
  - Sources: https://www.postgresql.org/developer/roadmap/ (Authoritative, read 2026-09-12); https://www.postgresql.org/developer/beta/ (Authoritative, read 2026-09-12); https://www.postgresql.org/about/news/postgresql-19-beta-1-released-3313/ (Authoritative, 2026-06-04); https://www.postgresql.org/about/news/postgresql-19-beta-2-released-3350/ (Authoritative, 2026-07-16); https://wiki.postgresql.org/wiki/PostgreSQL_19_Open_Items (Authoritative project wiki, read 2026-09-12); https://www.postgresql.org/docs/19/release-19.html (Authoritative, notes as of 2026-07-18).
  - Confidence: High.
- Finding 1.5: Minor release calendar.
  - Claim: the next scheduled minor release is November 12, 2026, then February 11, 2027, May 13, 2027, August 12, 2027.
  - Evidence quote: "The target date for these releases are, unless otherwise stated, the second Thursday of February, May, August, and November. The current schedule for upcoming releases is: November 12th, 2026, February 11th, 2027, May 13th, 2027, August 12th, 2027".
  - Source: https://www.postgresql.org/developer/roadmap/ (Authoritative, read 2026-09-12).
  - Confidence: High.

### Q2. PostgreSQL 18, 17, and 19 headline features

- Finding 2.1: PostgreSQL 18 overview list (official).
  - Evidence quote (E.6.1 Overview): "An asynchronous I/O (AIO) subsystem that can improve performance of sequential scans, bitmap heap scans, vacuums, and other operations."; "pg_upgrade now retains optimizer statistics."; "Support for "skip scan" lookups that allow using multicolumn B-tree indexes in more cases."; "uuidv7() function for generating timestamp-ordered UUIDs."; "Virtual generated columns that compute their values during read operations. This is now the default for generated columns."; "OAuth authentication support."; "OLD and NEW support for RETURNING clauses in INSERT, UPDATE, DELETE, and MERGE commands."; "Temporal constraints, or constraints over ranges, for PRIMARY KEY, UNIQUE, and FOREIGN KEY constraints."
  - Source: https://www.postgresql.org/docs/18/release-18.html (Authoritative, 2025-09-25).
  - Confidence: High.
- Finding 2.2: PostgreSQL 18 details the brief asked for, each quoted from the same release notes page.
  - Asynchronous I/O: "This feature allows backends to queue multiple read requests, which allows for more efficient sequential scans, bitmap heap scans, vacuums, etc. This is enabled by server variable io_method, with server variables io_combine_limit and io_max_combine_limit added to control it. ... The new system view pg_aios shows the file handles being used for asynchronous I/O."
  - Skip scan: "Allow skip scans of btree indexes (Peter Geoghegan)".
  - uuidv7: "Add UUID version 7 generation function uuidv7() (Andrey Borodin)"; "This UUID value is temporally sortable. Function alias uuidv4() has been added to explicitly generate version 4 UUIDs."
  - Virtual generated columns: "Allow generated columns to be virtual, and make them the default"; "Virtual generated columns generate their values when the columns are read, not written. The write behavior can still be specified via the STORED option."
  - OAuth: "This adds an oauth authentication method to pg_hba.conf, libpq OAuth options, a server variable oauth_validator_libraries to load token validation libraries, and a configure flag --with-libcurl".
  - RETURNING OLD/NEW: "This new syntax allows the RETURNING list of INSERT/UPDATE/DELETE/MERGE to explicitly return old and new values by using the special aliases old and new. These aliases can be renamed to avoid identifier conflicts."
  - Temporal constraints: "This is specified by WITHOUT OVERLAPS for PRIMARY KEY and UNIQUE, and by PERIOD for foreign keys, all applied to the last specified column."
  - NOT ENFORCED: "Allow CHECK and foreign key constraints to be specified as NOT ENFORCED (Amul Sul)"; "This also adds column pg_constraint.conenforced."
  - NOT NULL as constraints: "Store column NOT NULL specifications in pg_constraint"; "This allows names to be specified for NOT NULL constraint."; "Allow ALTER TABLE to set the NOT VALID attribute of NOT NULL constraints".
  - Data checksums default: "Change initdb default to enable data checksums (Greg Sabino Mullane)"; "Checksums can be disabled with the new initdb option --no-data-checksums. pg_upgrade requires matching cluster checksum settings".
  - MD5 deprecation: "Deprecate MD5 password authentication (Nathan Bossart)"; "Support for MD5 passwords will be removed in a future major version release. CREATE ROLE and ALTER ROLE now emit deprecation warnings when setting MD5 passwords. These warnings can be disabled by setting the md5_password_warnings parameter to off."
  - pg_upgrade: "Allow pg_upgrade to preserve optimizer statistics"; "Extended statistics are not preserved. Also add pg_upgrade option --no-statistics"; "Allow pg_upgrade to process database checks in parallel ... controlled by the existing --jobs option"; "Add pg_upgrade option --swap to swap directories rather than copy, clone, or link files"; "Add pg_upgrade option --set-char-signedness".
  - New or changed statistics: "Modify pg_stat_all_tables and its variants to report the time spent in VACUUM, ANALYZE, and their automatic variants ... total_vacuum_time, total_autovacuum_time, total_analyze_time, and total_autoanalyze_time"; "Add per-backend I/O statistics reporting ... accessed via pg_stat_get_backend_io()"; "Add pg_stat_io columns to report I/O activity in bytes ... read_bytes, write_bytes, and extend_bytes. The op_bytes column ... has been removed"; "Add WAL I/O activity rows to pg_stat_io"; "Remove read/sync columns from pg_stat_wal ... wal_write, wal_sync, wal_write_time, and wal_sync_time"; "Add column pg_stat_checkpointer.num_done"; "Add column pg_stat_checkpointer.slru_written"; "Add columns to pg_stat_database to report parallel worker activity ... parallel_workers_to_launch and parallel_workers_launched".
  - EXPLAIN: "Automatically include BUFFERS output in EXPLAIN ANALYZE"; "Add full WAL buffer count to EXPLAIN (WAL) output"; "In EXPLAIN ANALYZE, report the number of index lookups used per index scan node"; "Modify EXPLAIN to output fractional row counts".
  - Other notable items: "Add function casefold()"; "Add REJECT_LIMIT to control the number of invalid rows COPY FROM can ignore"; "Allow COPY TO to copy rows from populated materialized views"; "Allow NOT VALID foreign key constraints on partitioned tables"; "Add pg_dump option --statistics"; "Add pg_dump, pg_dumpall, and pg_restore options --statistics-only, --no-statistics, --no-data, and --no-schema"; "Add option --no-policies"; "Add pg_createsubscriber option --all"; "Add vacuumdb option --missing-stats-only"; "Add extension pg_overexplain"; "Allow the values of generated columns to be logically replicated".
  - Source: https://www.postgresql.org/docs/18/release-18.html (Authoritative, 2025-09-25), corroborated by the press kit https://www.postgresql.org/about/press/presskit18/en/ (Authoritative, 2025-09-25) which adds "This release deprecates md5 password authentication" and "PostgreSQL 18 makes it easier to create the schema definition of a foreign table using the definition of a local table with the CREATE FOREIGN TABLE ... LIKE command."
  - Confidence: High.
- Finding 2.3: PostgreSQL 17 headlines.
  - Evidence quotes: "Release date: 2024-09-26"; "New SQL/JSON capabilities, including constructors, identity functions, and the JSON_TABLE() function, which converts JSON data into a table representation."; "pg_basebackup now supports incremental backup."; "Add application pg_createsubscriber to create a logical replica from a physical standby server"; "Add support for incremental file system backup ... pg_basebackup's new --incremental option. The new application pg_combinebackup"; "Allow MERGE to use the RETURNING clause (Dean Rasheed)"; "The new RETURNING function merge_action() reports on the DML that generated the row."; "Add WHEN NOT MATCHED BY SOURCE to MERGE"; "Allow MERGE to modify updatable views"; "Create system view pg_stat_checkpointer"; "Add EXPLAIN option SERIALIZE"; "Allow EXPLAIN to report optimizer memory usage"; "The permission can be granted on a per-table basis using the MAINTAIN privilege and on a per-role basis via the pg_maintain predefined role."; "Change functions to use a safe search_path during maintenance operations".
  - Source: https://www.postgresql.org/docs/17/release-17.html (Authoritative, 2024-09-26).
  - Confidence: High.
- Finding 2.4: PostgreSQL 19 beta headlines (official notes, subject to change).
  - Evidence quotes (E.1.1 Overview): "Support for property graph queries (SQL/PGQ)."; "A new REPACK command that reclaims disk space and reorganizes table contents, combining the functionality of the existing VACUUM FULL and CLUSTER commands. Its CONCURRENTLY option allows repacking without blocking reads and writes to the table."; "Logical replication now replicates sequence values and can be enabled without a server restart when wal_level is set to replica."; "Autovacuum can now use parallel worker processes to vacuum a table's indexes, and a new scoring system prioritizes the tables that most need vacuuming or analyzing."; "Data checksums can now be enabled or disabled while the database server is running."; "A new WAIT FOR command that waits until a standby has replayed changes up to a chosen point"; "Support for temporal updates and deletes via the new FOR PORTION OF clause."; "A new pg_plan_advice extension for stabilizing and controlling the query planner's decisions, and a companion pg_stash_advice extension".
  - Incompatibilities quoted: "Issue a warning after successful MD5 password authentication"; "Remove RADIUS support"; "Force standard_conforming_strings to always be on in the database server"; "Change default of max_locks_per_transaction from 64 to 128"; "Change JIT to be disabled by default"; "Change the default TOAST compression method from pglz to the more efficient lz4"; "Enable server variable log_lock_waits by default".
  - Developer-facing quoted: "Add support for INSERT ... ON CONFLICT DO SELECT ... RETURNING"; "Allow window functions to ignore NULLs with the IGNORE NULLS/RESPECT NULLS clause"; "Allow COPY TO to output JSON format"; "Allow COPY FROM to set invalid input values to NULL ... ON_ERROR SET_NULL"; "Add EXPLAIN ANALYZE option IO"; "Allow ALTER TABLE ALTER CONSTRAINT ... [NOT] ENFORCED for CHECK constraints"; "Add function pg_get_role_ddl()", "pg_get_tablespace_ddl()", "pg_get_database_ddl()"; "Allow GRANT/REVOKE to specify the effective role ... GRANTED BY"; "Allow CHECKPOINT to accept a list of options ... MODE and FLUSH_UNLOGGED"; "Add system view pg_stat_lock", "pg_stat_recovery", "pg_stat_autovacuum_scores"; "Allow roles pg_read_all_data and pg_write_all_data to read/write large objects"; "Allow vacuumdb to report its commands without running them using option --dry-run"; "Add the 64-bit unsigned data type oid8".
  - Beta 3 changes quoted from the announcement: "Revert GROUP BY ALL."; "Several fixes for the new FOR PORTION OF temporal table syntax."; "Several fixes for the new logical replication sequence synchronization feature".
  - Sources: https://www.postgresql.org/docs/19/release-19.html (Authoritative, "AS OF 2026-07-18", read 2026-09-12); https://www.postgresql.org/about/news/postgresql-186-1711-1615-1519-1424-and-19-beta-3-released-3365/ (Authoritative, 2026-08-13).
  - Confidence: High for "what the beta contains as documented"; the content is explicitly subject to change before GA.
- Finding 2.5: MERGE/SPLIT PARTITIONS is gone from 19 despite the notes. See Conflicts, section 3.

### Q3. DDL surface a DBA tool must cover (PostgreSQL 18 docs)

Every synopsis below was extracted from the `<pre class="synopsis">` block of the named PostgreSQL 18 reference page on 2026-09-12 (Authoritative, docs for 18.6). Confidence: High for all synopses.

- Finding 3.1: Databases. https://www.postgresql.org/docs/18/sql-createdatabase.html

```sql
CREATE DATABASE name
    [ WITH ] [ OWNER [=] user_name ] [ TEMPLATE [=] template ] [ ENCODING [=] encoding ]
           [ STRATEGY [=] strategy ] [ LOCALE [=] locale ] [ LC_COLLATE [=] lc_collate ] [ LC_CTYPE [=] lc_ctype ]
           [ BUILTIN_LOCALE [=] builtin_locale ] [ ICU_LOCALE [=] icu_locale ] [ ICU_RULES [=] icu_rules ]
           [ LOCALE_PROVIDER [=] locale_provider ] [ COLLATION_VERSION = collation_version ]
           [ TABLESPACE [=] tablespace_name ] [ ALLOW_CONNECTIONS [=] allowconn ] [ CONNECTION LIMIT [=] connlimit ]
           [ IS_TEMPLATE [=] istemplate ] [ OID [=] oid ]
```

  - Note for a one-database-per-run tool: CREATE DATABASE and DROP DATABASE cannot run inside a transaction block, and a session cannot drop the database it is connected to.

- Finding 3.2: Schemas. https://www.postgresql.org/docs/18/sql-createschema.html

```sql
CREATE SCHEMA schema_name [ AUTHORIZATION role_specification ] [ schema_element [ ... ] ]
CREATE SCHEMA IF NOT EXISTS schema_name [ AUTHORIZATION role_specification ]
```

  - Public schema rule (https://www.postgresql.org/docs/18/ddl-schemas.html): "In PostgreSQL 15 and later, the default configuration supports this usage pattern. In prior versions, or when using a database that has been upgraded from a prior version, you will need to remove the public CREATE privilege from the public schema (issue REVOKE CREATE ON SCHEMA public FROM PUBLIC)."

- Finding 3.3: Tables, including partitioned, unlogged, temporary, inheritance, LIKE, generated and identity columns. https://www.postgresql.org/docs/18/sql-createtable.html

```sql
CREATE [ [ GLOBAL | LOCAL ] { TEMPORARY | TEMP } | UNLOGGED ] TABLE [ IF NOT EXISTS ] table_name ( [
  { column_name data_type [ STORAGE { PLAIN | EXTERNAL | EXTENDED | MAIN | DEFAULT } ] [ COMPRESSION compression_method ] [ COLLATE collation ] [ column_constraint [ ... ] ]
    | table_constraint
    | LIKE source_table [ like_option ... ] }
    [, ... ]
] )
[ INHERITS ( parent_table [, ... ] ) ]
[ PARTITION BY { RANGE | LIST | HASH } ( { column_name | ( expression ) } [ COLLATE collation ] [ opclass ] [, ... ] ) ]
[ USING method ]
[ WITH ( storage_parameter [= value] [, ... ] ) | WITHOUT OIDS ]
[ ON COMMIT { PRESERVE ROWS | DELETE ROWS | DROP } ]
[ TABLESPACE tablespace_name ]

CREATE ... TABLE [ IF NOT EXISTS ] table_name
    PARTITION OF parent_table [ ( ... ) ] { FOR VALUES partition_bound_spec | DEFAULT }

where column_constraint is:
[ CONSTRAINT constraint_name ]
{ NOT NULL [ NO INHERIT ] | NULL | CHECK ( expression ) [ NO INHERIT ] | DEFAULT default_expr |
  GENERATED ALWAYS AS ( generation_expr ) [ STORED | VIRTUAL ] |
  GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY [ ( sequence_options ) ] |
  UNIQUE [ NULLS [ NOT ] DISTINCT ] index_parameters | PRIMARY KEY index_parameters |
  REFERENCES reftable [ ( refcolumn ) ] [ MATCH FULL | MATCH PARTIAL | MATCH SIMPLE ]
    [ ON DELETE referential_action ] [ ON UPDATE referential_action ] }
[ DEFERRABLE | NOT DEFERRABLE ] [ INITIALLY DEFERRED | INITIALLY IMMEDIATE ] [ ENFORCED | NOT ENFORCED ]

and like_option is:
{ INCLUDING | EXCLUDING } { COMMENTS | COMPRESSION | CONSTRAINTS | DEFAULTS | GENERATED | IDENTITY | INDEXES | STATISTICS | STORAGE | ALL }
```

  - Generated columns in 18 (https://www.postgresql.org/docs/18/ddl-generated-columns.html): "A generated column is by default of the virtual kind. Use the keywords VIRTUAL or STORED to make the choice explicit." and "A virtual generated column cannot have a user-defined type, and the generation expression of a virtual generated column must not reference user-defined functions or types".

- Finding 3.4: Columns and constraints via ALTER TABLE. https://www.postgresql.org/docs/18/sql-altertable.html

```sql
ALTER TABLE [ IF EXISTS ] [ ONLY ] name [ * ] action [, ... ]
ALTER TABLE [ IF EXISTS ] [ ONLY ] name [ * ] RENAME [ COLUMN ] column_name TO new_column_name
ALTER TABLE [ IF EXISTS ] [ ONLY ] name [ * ] RENAME CONSTRAINT constraint_name TO new_constraint_name
ALTER TABLE [ IF EXISTS ] name RENAME TO new_name
ALTER TABLE [ IF EXISTS ] name SET SCHEMA new_schema
ALTER TABLE [ IF EXISTS ] name ATTACH PARTITION partition_name { FOR VALUES partition_bound_spec | DEFAULT }
ALTER TABLE [ IF EXISTS ] name DETACH PARTITION partition_name [ CONCURRENTLY | FINALIZE ]

where action is one of:
    ADD [ COLUMN ] [ IF NOT EXISTS ] column_name data_type [ COLLATE collation ] [ column_constraint [ ... ] ]
    DROP [ COLUMN ] [ IF EXISTS ] column_name [ RESTRICT | CASCADE ]
    ALTER [ COLUMN ] column_name [ SET DATA ] TYPE data_type [ COLLATE collation ] [ USING expression ]
    ALTER [ COLUMN ] column_name SET DEFAULT expression
    ALTER [ COLUMN ] column_name DROP DEFAULT
    ALTER [ COLUMN ] column_name { SET | DROP } NOT NULL
    ALTER [ COLUMN ] column_name SET EXPRESSION AS ( expression )
    ALTER [ COLUMN ] column_name DROP EXPRESSION [ IF EXISTS ]
    ALTER [ COLUMN ] column_name ADD GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY [ ( sequence_options ) ]
    ALTER [ COLUMN ] column_name { SET GENERATED { ALWAYS | BY DEFAULT } | SET sequence_option | RESTART [ [ WITH ] restart ] } [...]
    ALTER [ COLUMN ] column_name DROP IDENTITY [ IF EXISTS ]
    ALTER [ COLUMN ] column_name SET STATISTICS { integer | DEFAULT }
    ALTER [ COLUMN ] column_name SET STORAGE { PLAIN | EXTERNAL | EXTENDED | MAIN | DEFAULT }
    ALTER [ COLUMN ] column_name SET COMPRESSION compression_method
    ADD table_constraint [ NOT VALID ]
    ADD table_constraint_using_index
    ALTER CONSTRAINT constraint_name [ DEFERRABLE | NOT DEFERRABLE ] [ INITIALLY DEFERRED | INITIALLY IMMEDIATE ] [ ENFORCED | NOT ENFORCED ]
    ALTER CONSTRAINT constraint_name [ INHERIT | NO INHERIT ]
    VALIDATE CONSTRAINT constraint_name
    DROP CONSTRAINT [ IF EXISTS ]  constraint_name [ RESTRICT | CASCADE ]
    DISABLE TRIGGER [ trigger_name | ALL | USER ] | ENABLE TRIGGER [ trigger_name | ALL | USER ]
    ENABLE REPLICA TRIGGER trigger_name | ENABLE ALWAYS TRIGGER trigger_name
    DISABLE ROW LEVEL SECURITY | ENABLE ROW LEVEL SECURITY | FORCE ROW LEVEL SECURITY | NO FORCE ROW LEVEL SECURITY
    CLUSTER ON index_name | SET WITHOUT CLUSTER
    SET ACCESS METHOD { new_access_method | DEFAULT }
    SET TABLESPACE new_tablespace
    SET { LOGGED | UNLOGGED }
    SET ( storage_parameter [= value] [, ... ] ) | RESET ( storage_parameter [, ... ] )
    INHERIT parent_table | NO INHERIT parent_table
    OWNER TO { new_owner | CURRENT_ROLE | CURRENT_USER | SESSION_USER }
    REPLICA IDENTITY { DEFAULT | USING INDEX index_name | FULL | NOTHING }
```

  - Table constraint grammar (same page and CREATE TABLE): "CHECK ( expression ) [ NO INHERIT ] | NOT NULL column_name [ NO INHERIT ] | UNIQUE [ NULLS [ NOT ] DISTINCT ] ( column_name [, ... ] [, column_name WITHOUT OVERLAPS ] ) index_parameters | PRIMARY KEY ( column_name [, ... ] [, column_name WITHOUT OVERLAPS ] ) index_parameters | EXCLUDE [ USING index_method ] ( exclude_element WITH operator [, ... ] ) index_parameters [ WHERE ( predicate ) ] | FOREIGN KEY ( column_name [, ... ] [, PERIOD column_name ] ) REFERENCES reftable [ ( refcolumn [, ... ] [, PERIOD refcolumn ] ) ] [ MATCH FULL | MATCH PARTIAL | MATCH SIMPLE ] [ ON DELETE referential_action ] [ ON UPDATE referential_action ]" followed by "[ DEFERRABLE | NOT DEFERRABLE ] [ INITIALLY DEFERRED | INITIALLY IMMEDIATE ] [ ENFORCED | NOT ENFORCED ]".
  - NOT VALID then VALIDATE, quoted: "This form adds a new constraint to a table using the same constraint syntax as CREATE TABLE, plus the option NOT VALID, which is currently only allowed for foreign-key, CHECK, and not-null constraints." and "validation acquires only a SHARE UPDATE EXCLUSIVE lock on the table being altered. (If the constraint is a foreign key then a ROW SHARE lock is also required on the table referenced by the constraint.)" and "If the constraint was set to NOT ENFORCED, an error is thrown."
  - NOT ENFORCED in 18, quoted from CREATE TABLE: "If the constraint is NOT ENFORCED, the database system will not check the constraint. It is then up to the application code to ensure that the constraints are satisfied. ... This is currently only supported for foreign key and CHECK constraints." The ALTER CONSTRAINT enforceability switch applies to foreign keys in 18; the 19 notes say "Allow ALTER TABLE ALTER CONSTRAINT ... [NOT] ENFORCED for CHECK constraints ... Previously enforcement changes were only supported for foreign key constraints." (https://www.postgresql.org/docs/19/release-19.html, 2026-07-18). The commit message for the FK part says "when creating a NOT ENFORCED foreign key constraint, triggers will not be created, and the constraint will be marked as NOT VALID" (https://www.postgresql.org/message-id/E1tzwW2-002HaO-0C%40gemulon.postgresql.org, 2025-04-02).
  - Deferrable rule, quoted: "Currently, only UNIQUE, PRIMARY KEY, EXCLUDE, and REFERENCES (foreign key) constraints accept this clause. NOT NULL and CHECK constraints are not deferrable. Note that deferrable constraints cannot be used as conflict arbiters in an INSERT statement that includes an ON CONFLICT clause."
  - Foreign key actions, quoted from https://www.postgresql.org/docs/18/ddl-constraints.html: "The default ON DELETE action is ON DELETE NO ACTION"; "RESTRICT is a stricter setting than NO ACTION. It prevents deletion of a referenced row. RESTRICT does not allow the check to be deferred"; "There is also a noticeable difference between ON UPDATE NO ACTION (the default) and ON UPDATE RESTRICT."; SET NULL and SET DEFAULT also exist and "column lists cannot be specified for SET NULL and SET DEFAULT" on ON UPDATE.
  - Canonical two-step add:

```sql
ALTER TABLE distributors ADD CONSTRAINT distfk FOREIGN KEY (address) REFERENCES addresses (address) NOT VALID;
ALTER TABLE distributors VALIDATE CONSTRAINT distfk;
```

- Finding 3.5: Indexes. https://www.postgresql.org/docs/18/sql-createindex.html and https://www.postgresql.org/docs/18/sql-reindex.html

```sql
CREATE [ UNIQUE ] INDEX [ CONCURRENTLY ] [ [ IF NOT EXISTS ] name ] ON [ ONLY ] table_name [ USING method ]
    ( { column_name | ( expression ) } [ COLLATE collation ] [ opclass [ ( opclass_parameter = value [, ... ] ) ] ] [ ASC | DESC ] [ NULLS { FIRST | LAST } ] [, ...] )
    [ INCLUDE ( column_name [, ...] ) ]
    [ NULLS [ NOT ] DISTINCT ]
    [ WITH ( storage_parameter [= value] [, ... ] ) ]
    [ TABLESPACE tablespace_name ]
    [ WHERE predicate ]

REINDEX [ ( option [, ...] ) ] { INDEX | TABLE | SCHEMA } [ CONCURRENTLY ] name
REINDEX [ ( option [, ...] ) ] { DATABASE | SYSTEM } [ CONCURRENTLY ] [ name ]
where option can be one of: CONCURRENTLY [ boolean ] | TABLESPACE new_tablespace | VERBOSE [ boolean ]
```

  - Methods: the "USING method" slot takes btree, hash, gist, spgist, gin, brin (the CREATE INDEX page lists these). Rules a tool must honor, quoted: "a regular CREATE INDEX command can be performed within a transaction block, but CREATE INDEX CONCURRENTLY cannot."; "If a problem arises while scanning the table ... the CREATE INDEX command will fail but leave behind an "invalid" index ... The psql \d command will report such an index as INVALID"; "The recommended recovery method in such cases is to drop the index and try again to perform CREATE INDEX CONCURRENTLY. (Another possibility is to rebuild the index with REINDEX INDEX CONCURRENTLY)." REINDEX: "a regular REINDEX TABLE or REINDEX INDEX command can be performed within a transaction block, but REINDEX CONCURRENTLY cannot."; invalid leftovers are suffixed "_ccnew" or "_ccold" (https://www.postgresql.org/docs/17/sql-reindex.html text, identical in 18).
  - Confidence: High.

- Finding 3.6: Views and materialized views. https://www.postgresql.org/docs/18/sql-createview.html, https://www.postgresql.org/docs/18/sql-creatematerializedview.html, https://www.postgresql.org/docs/18/sql-refreshmaterializedview.html

```sql
CREATE [ OR REPLACE ] [ TEMP | TEMPORARY ] [ RECURSIVE ] VIEW name [ ( column_name [, ...] ) ]
    [ WITH ( view_option_name [= view_option_value] [, ... ] ) ]
    AS query
    [ WITH [ CASCADED | LOCAL ] CHECK OPTION ]

REFRESH MATERIALIZED VIEW [ CONCURRENTLY ] name [ WITH [ NO ] DATA ]
```

  - Quoted: "To execute this command you must have the MAINTAIN privilege on the materialized view."; CONCURRENTLY "is only allowed if there is at least one UNIQUE index on the materialized view which uses only column names and includes all rows; that is, it must not be an expression index or include a WHERE clause."

- Finding 3.7: Sequences. https://www.postgresql.org/docs/18/sql-createsequence.html

```sql
CREATE [ { TEMPORARY | TEMP } | UNLOGGED ] SEQUENCE [ IF NOT EXISTS ] name
    [ AS data_type ] [ INCREMENT [ BY ] increment ]
    [ MINVALUE minvalue | NO MINVALUE ] [ MAXVALUE maxvalue | NO MAXVALUE ]
    [ [ NO ] CYCLE ] [ START [ WITH ] start ] [ CACHE cache ]
    [ OWNED BY { table_name.column_name | NONE } ]
```

- Finding 3.8: Functions and procedures. https://www.postgresql.org/docs/18/sql-createfunction.html and https://www.postgresql.org/docs/18/sql-createprocedure.html

```sql
CREATE [ OR REPLACE ] FUNCTION
    name ( [ [ argmode ] [ argname ] argtype [ { DEFAULT | = } default_expr ] [, ...] ] )
    [ RETURNS rettype | RETURNS TABLE ( column_name column_type [, ...] ) ]
  { LANGUAGE lang_name | TRANSFORM { FOR TYPE type_name } [, ... ] | WINDOW
    | { IMMUTABLE | STABLE | VOLATILE } | [ NOT ] LEAKPROOF
    | { CALLED ON NULL INPUT | RETURNS NULL ON NULL INPUT | STRICT }
    | { [ EXTERNAL ] SECURITY INVOKER | [ EXTERNAL ] SECURITY DEFINER }
    | PARALLEL { UNSAFE | RESTRICTED | SAFE } | COST execution_cost | ROWS result_rows | SUPPORT support_function
    | SET configuration_parameter { TO value | = value | FROM CURRENT }
    | AS 'definition' | AS 'obj_file', 'link_symbol' | sql_body } ...

CREATE [ OR REPLACE ] PROCEDURE
    name ( [ [ argmode ] [ argname ] argtype [ { DEFAULT | = } default_expr ] [, ...] ] )
  { LANGUAGE lang_name | TRANSFORM { FOR TYPE type_name } [, ... ]
    | [ EXTERNAL ] SECURITY INVOKER | [ EXTERNAL ] SECURITY DEFINER
    | SET configuration_parameter { TO value | = value | FROM CURRENT }
    | AS 'definition' | AS 'obj_file', 'link_symbol' | sql_body } ...
```

  - SECURITY DEFINER rule, quoted: "Because a SECURITY DEFINER function is executed with the privileges of the user that owns it, care is needed to ensure that the function cannot be misused. For security, search_path should be set to exclude any schemas writable by untrusted users. ... A secure arrangement can be obtained by forcing the temporary schema to be searched last. To do this, write pg_temp as the last entry in search_path." The doc's own example: "SET search_path = admin, pg_temp;". LANGUAGE sql and LANGUAGE plpgsql are the two languages a tool should expose by default; sql_body is the SQL-standard BEGIN ATOMIC form.

- Finding 3.9: Triggers and event triggers. https://www.postgresql.org/docs/18/sql-createtrigger.html and https://www.postgresql.org/docs/18/sql-createeventtrigger.html

```sql
CREATE [ OR REPLACE ] [ CONSTRAINT ] TRIGGER name { BEFORE | AFTER | INSTEAD OF } { event [ OR ... ] }
    ON table_name [ FROM referenced_table_name ]
    [ NOT DEFERRABLE | [ DEFERRABLE ] [ INITIALLY IMMEDIATE | INITIALLY DEFERRED ] ]
    [ REFERENCING { { OLD | NEW } TABLE [ AS ] transition_relation_name } [ ... ] ]
    [ FOR [ EACH ] { ROW | STATEMENT } ] [ WHEN ( condition ) ]
    EXECUTE { FUNCTION | PROCEDURE } function_name ( arguments )
where event can be one of: INSERT | UPDATE [ OF column_name [, ... ] ] | DELETE | TRUNCATE

CREATE EVENT TRIGGER name ON event
    [ WHEN filter_variable IN (filter_value [, ... ]) [ AND ... ] ]
    EXECUTE { FUNCTION | PROCEDURE } function_name()
```

- Finding 3.10: Types: enum, composite, domain, range. https://www.postgresql.org/docs/18/sql-createtype.html, https://www.postgresql.org/docs/18/sql-altertype.html, https://www.postgresql.org/docs/18/sql-createdomain.html

```sql
CREATE TYPE name AS ( [ attribute_name data_type [ COLLATE collation ] [, ... ] ] )
CREATE TYPE name AS ENUM ( [ 'label' [, ... ] ] )
CREATE TYPE name AS RANGE ( SUBTYPE = subtype [ , SUBTYPE_OPCLASS = subtype_operator_class ] [ , COLLATION = collation ]
    [ , CANONICAL = canonical_function ] [ , SUBTYPE_DIFF = subtype_diff_function ] [ , MULTIRANGE_TYPE_NAME = multirange_type_name ] )
CREATE DOMAIN name [ AS ] data_type [ COLLATE collation ] [ DEFAULT expression ] [ domain_constraint [ ... ] ]
ALTER TYPE name ADD VALUE [ IF NOT EXISTS ] new_enum_value [ { BEFORE | AFTER } neighbor_enum_value ]
ALTER TYPE name RENAME VALUE existing_enum_value TO new_enum_value
ALTER TYPE name ADD ATTRIBUTE attribute_name data_type [ COLLATE collation ] [ CASCADE | RESTRICT ]
```

  - Enum limits, quoted: "If ALTER TYPE ... ADD VALUE (the form that adds a new value to an enum type) is executed inside a transaction block, the new value cannot be used until after the transaction has been committed." and (datatype-enum) "Existing values cannot be removed from an enum type, nor can the sort ordering of such values be changed, short of dropping and re-creating the enum type." A tool must run ADD VALUE outside the transaction that first uses the value, or it will get "unsafe use of new value" (SQLSTATE 55P04).

- Finding 3.11: Extensions. https://www.postgresql.org/docs/18/sql-createextension.html and https://www.postgresql.org/docs/18/view-pg-available-extensions.html

```sql
CREATE EXTENSION [ IF NOT EXISTS ] extension_name [ WITH ] [ SCHEMA schema_name ] [ VERSION version ] [ CASCADE ]
SELECT name, default_version, installed_version, comment FROM pg_available_extensions ORDER BY 1;
```

  - Quoted: "if the extension is marked trusted in its control file, then it can be installed by any user who has CREATE privilege on the current database."

- Finding 3.12: Roles and privileges. https://www.postgresql.org/docs/18/sql-createrole.html, https://www.postgresql.org/docs/18/sql-grant.html, https://www.postgresql.org/docs/18/sql-revoke.html, https://www.postgresql.org/docs/18/sql-alterdefaultprivileges.html

```sql
CREATE ROLE name [ [ WITH ] option [ ... ] ]
where option can be: SUPERUSER | NOSUPERUSER | CREATEDB | NOCREATEDB | CREATEROLE | NOCREATEROLE | INHERIT | NOINHERIT
    | LOGIN | NOLOGIN | REPLICATION | NOREPLICATION | BYPASSRLS | NOBYPASSRLS | CONNECTION LIMIT connlimit
    | [ ENCRYPTED ] PASSWORD 'password' | PASSWORD NULL | VALID UNTIL 'timestamp'
    | IN ROLE role_name [, ...] | ROLE role_name [, ...] | ADMIN role_name [, ...] | SYSID uid

GRANT { { SELECT | INSERT | UPDATE | DELETE | TRUNCATE | REFERENCES | TRIGGER | MAINTAIN } [, ...] | ALL [ PRIVILEGES ] }
    ON { [ TABLE ] table_name [, ...] | ALL TABLES IN SCHEMA schema_name [, ...] }
    TO role_specification [, ...] [ WITH GRANT OPTION ] [ GRANTED BY role_specification ]
GRANT { { USAGE | SELECT | UPDATE } [, ...] | ALL [ PRIVILEGES ] } ON { SEQUENCE ... | ALL SEQUENCES IN SCHEMA ... } TO ...
GRANT { { CREATE | USAGE } [, ...] | ALL [ PRIVILEGES ] } ON SCHEMA schema_name [, ...] TO ...
GRANT { EXECUTE | ALL [ PRIVILEGES ] } ON { { FUNCTION | PROCEDURE | ROUTINE } routine_name [ ( ... ) ] [, ...] | ALL { FUNCTIONS | PROCEDURES | ROUTINES } IN SCHEMA ... } TO ...
GRANT { { CREATE | CONNECT | TEMPORARY | TEMP } [, ...] | ALL [ PRIVILEGES ] } ON DATABASE database_name [, ...] TO ...
GRANT { { SET | ALTER SYSTEM } [, ... ] | ALL [ PRIVILEGES ] } ON PARAMETER configuration_parameter [, ...] TO ...
GRANT role_name [, ...] TO role_specification [, ...] [ WITH { ADMIN | INHERIT | SET } { OPTION | TRUE | FALSE } ] [ GRANTED BY role_specification ]

ALTER DEFAULT PRIVILEGES [ FOR { ROLE | USER } target_role [, ...] ] [ IN SCHEMA schema_name [, ...] ] abbreviated_grant_or_revoke
```

  - Role membership options, quoted from GRANT: "The ADMIN option allows the member to in turn grant membership in the role to others, and revoke membership in the role as well. ... This option defaults to FALSE."; "The INHERIT option controls the inheritance status of the new membership ... If unspecified when creating a new role membership, this defaults to the inheritance attribute of the new member."; "The SET option, if it is set to TRUE, allows the member to change to the granted role using the SET ROLE command. ... This option defaults to TRUE."
  - ALTER DEFAULT PRIVILEGES scope, quoted: "Currently, only the privileges for schemas, tables (including views and foreign tables), sequences, functions, types (including domains), and large objects can be altered." and "per-schema default privileges can only add privileges to the global setting, not remove privileges granted by it."

- Finding 3.13: Row-level security. https://www.postgresql.org/docs/18/sql-createpolicy.html and https://www.postgresql.org/docs/18/ddl-rowsecurity.html

```sql
CREATE POLICY name ON table_name
    [ AS { PERMISSIVE | RESTRICTIVE } ]
    [ FOR { ALL | SELECT | INSERT | UPDATE | DELETE } ]
    [ TO { role_name | PUBLIC | CURRENT_ROLE | CURRENT_USER | SESSION_USER } [, ...] ]
    [ USING ( using_expression ) ]
    [ WITH CHECK ( check_expression ) ]
ALTER TABLE t ENABLE ROW LEVEL SECURITY;
```

  - Quoted: "If no policy exists for the table, a default-deny policy is used, meaning that no rows are visible or can be modified. Operations that apply to the whole table, such as TRUNCATE and REFERENCES, are not subject to row security." and "You must be the owner of a table to create or change policies for it."

- Finding 3.14: COMMENT ON. https://www.postgresql.org/docs/18/sql-comment.html
  - The synopsis covers ACCESS METHOD, AGGREGATE, CAST, COLLATION, COLUMN, CONSTRAINT ... ON table or DOMAIN, CONVERSION, DATABASE, DOMAIN, EXTENSION, EVENT TRIGGER, FOREIGN DATA WRAPPER, FOREIGN TABLE, FUNCTION, INDEX, LARGE OBJECT, MATERIALIZED VIEW, OPERATOR, OPERATOR CLASS, OPERATOR FAMILY, POLICY, LANGUAGE, PROCEDURE, PUBLICATION, ROLE, ROUTINE, RULE, SCHEMA, SEQUENCE, SERVER, STATISTICS, SUBSCRIPTION, TABLE, TABLESPACE, TEXT SEARCH objects, TRANSFORM, TRIGGER, TYPE, VIEW, ending "IS { string_literal | NULL }". Quoted: "Specifying NULL or an empty string ('') removes the comment." and "There is presently no security mechanism for viewing comments: any user connected to a database can see all the comments".

- Finding 3.15: Tablespaces, publications, subscriptions, FDWs, collations.

```sql
CREATE TABLESPACE tablespace_name [ OWNER { new_owner | CURRENT_ROLE | CURRENT_USER | SESSION_USER } ] LOCATION 'directory' [ WITH ( tablespace_option = value [, ... ] ) ]
CREATE PUBLICATION name [ FOR ALL TABLES | FOR publication_object [, ... ] ] [ WITH ( publication_parameter [= value] [, ... ] ) ]
CREATE SUBSCRIPTION subscription_name CONNECTION 'conninfo' PUBLICATION publication_name [, ...] [ WITH ( subscription_parameter [= value] [, ... ] ) ]
CREATE FOREIGN DATA WRAPPER name [ HANDLER handler_function | NO HANDLER ] [ VALIDATOR validator_function | NO VALIDATOR ] [ OPTIONS ( option 'value' [, ... ] ) ]
CREATE SERVER [ IF NOT EXISTS ] server_name [ TYPE 'server_type' ] [ VERSION 'server_version' ] FOREIGN DATA WRAPPER fdw_name [ OPTIONS ( option 'value' [, ... ] ) ]
IMPORT FOREIGN SCHEMA remote_schema [ { LIMIT TO | EXCEPT } ( table_name [, ...] ) ] FROM SERVER server_name INTO local_schema [ OPTIONS ( option 'value' [, ... ] ) ]
CREATE COLLATION [ IF NOT EXISTS ] name ( [ LOCALE = locale, ] [ LC_COLLATE = lc_collate, ] [ LC_CTYPE = lc_ctype, ] [ PROVIDER = provider, ] [ DETERMINISTIC = boolean, ] [ RULES = rules, ] [ VERSION = version ] )
CREATE COLLATION [ IF NOT EXISTS ] name FROM existing_collation
```

  - Privilege notes quoted: tablespaces "Creation of the tablespace itself must be done as a database superuser" (https://www.postgresql.org/docs/18/manage-ag-tablespaces.html); publications "The FOR ALL TABLES and FOR TABLES IN SCHEMA clauses require the invoking user to be a superuser." and "The tables added to a publication that publishes UPDATE and/or DELETE operations must have REPLICA IDENTITY defined."; subscriptions "you must have the privileges of the pg_create_subscription role, as well as CREATE privileges on the current database." and "When creating a replication slot (the default behavior), CREATE SUBSCRIPTION cannot be executed inside a transaction block."; FDW "Only superusers can create foreign-data wrappers."; collation "CREATE COLLATION takes a SHARE ROW EXCLUSIVE lock, which is self-conflicting, on the pg_collation system catalog, so only one CREATE COLLATION command can run at a time."
  - Sources: the six PostgreSQL 18 pages named in the code block (Authoritative, docs for 18.6, read 2026-09-12).
  - Confidence: High.

### Q4. DML and query surface

- Finding 4.1: Pagination. LIMIT/OFFSET is in the SELECT synopsis; keyset uses a row-value comparison against a composite index.
  - Canonical keyset shape (Consensus sources agree; the row comparison syntax is standard PostgreSQL):

```sql
SELECT id, created_at, title
FROM posts
WHERE (created_at, id) < ($1::timestamptz, $2::uuid)
ORDER BY created_at DESC, id DESC
LIMIT 20;
-- with CREATE INDEX ON posts (created_at DESC, id DESC);
```

  - Evidence: "The rows skipped by an OFFSET clause still have to be computed inside the server; therefore a large OFFSET might be inefficient" is the PostgreSQL LIMIT/OFFSET rule the secondary sources quote; the seek pattern and the tie-breaker requirement are stated at https://blog.sequinstream.com/keyset-cursors-not-offsets-for-postgres-pagination/ (Consensus, 2024-12-04) and https://minervadb.com/optimizing-pagination-in-postgresql-17/ (Consensus, 2025-12-27). A tool should default to keyset for browsing and expose OFFSET only with a cap.
  - Confidence: Medium (pattern is consensus; the SELECT grammar itself is High).
- Finding 4.2: INSERT, UPDATE, DELETE, MERGE with RETURNING OLD/NEW (18 grammar, quoted synopses from https://www.postgresql.org/docs/18/sql-insert.html, sql-update.html, sql-delete.html, sql-merge.html):

```sql
INSERT INTO table_name [ AS alias ] [ ( column_name [, ...] ) ]
    [ OVERRIDING { SYSTEM | USER } VALUE ]
    { DEFAULT VALUES | VALUES ( { expression | DEFAULT } [, ...] ) [, ...] | query }
    [ ON CONFLICT [ conflict_target ] conflict_action ]
    [ RETURNING [ WITH ( { OLD | NEW } AS output_alias [, ...] ) ] { * | output_expression [ [ AS ] output_name ] } [, ...] ]
-- conflict_target: ( { index_column_name | ( index_expression ) } [ COLLATE collation ] [ opclass ] [, ...] ) [ WHERE index_predicate ] | ON CONSTRAINT constraint_name
-- conflict_action: DO NOTHING | DO UPDATE SET ... [ WHERE condition ]

UPDATE [ ONLY ] table_name [ * ] [ [ AS ] alias ] SET ... [ FROM from_item [, ...] ]
    [ WHERE condition | WHERE CURRENT OF cursor_name ]
    [ RETURNING [ WITH ( { OLD | NEW } AS output_alias [, ...] ) ] { * | output_expression [ [ AS ] output_name ] } [, ...] ]

DELETE FROM [ ONLY ] table_name [ * ] [ [ AS ] alias ] [ USING from_item [, ...] ]
    [ WHERE condition | WHERE CURRENT OF cursor_name ]
    [ RETURNING [ WITH ( { OLD | NEW } AS output_alias [, ...] ) ] { * | output_expression [ [ AS ] output_name ] } [, ...] ]

MERGE INTO [ ONLY ] target_table_name [ * ] [ [ AS ] target_alias ]
    USING data_source ON join_condition
    when_clause [...]
    [ RETURNING [ WITH ( { OLD | NEW } AS output_alias [, ...] ) ] { * | output_expression [ [ AS ] output_name ] } [, ...] ]
-- when_clause: WHEN MATCHED [ AND condition ] THEN { merge_update | merge_delete | DO NOTHING }
--            | WHEN NOT MATCHED BY SOURCE [ AND condition ] THEN { merge_update | merge_delete | DO NOTHING }
--            | WHEN NOT MATCHED [ BY TARGET ] [ AND condition ] THEN { merge_insert | DO NOTHING }
```

  - Example the release notes and press kit support: `UPDATE accounts SET balance = balance - 100 WHERE id = 42 RETURNING old.balance AS before, new.balance AS after;`
  - ON CONFLICT rule, quoted: "deferrable constraints cannot be used as conflict arbiters in an INSERT statement that includes an ON CONFLICT clause." (CREATE TABLE page).
  - Confidence: High.
- Finding 4.3: COPY over the wire and in tokio-postgres.
  - COPY grammar (https://www.postgresql.org/docs/18/sql-copy.html): "COPY table_name [ ( column_name [, ...] ) ] FROM { 'filename' | PROGRAM 'command' | STDIN } [ [ WITH ] ( option [, ...] ) ] [ WHERE condition ]" and "COPY { table_name [ ( column_name [, ...] ) ] | ( query ) } TO { 'filename' | PROGRAM 'command' | STDOUT } [ [ WITH ] ( option [, ...] ) ]" with options "FORMAT format_name, FREEZE, DELIMITER, NULL, DEFAULT, HEADER [ boolean | MATCH ], QUOTE, ESCAPE, FORCE_QUOTE, FORCE_NOT_NULL, FORCE_NULL, ON_ERROR error_action, REJECT_LIMIT maxerror, ENCODING, LOG_VERBOSITY verbosity". Quoted: "When STDIN or STDOUT is specified, data is transmitted via the connection between the client and the server." A tool must refuse the 'filename' and PROGRAM forms unless it deliberately wants server-side file access (those need pg_read_server_files, pg_write_server_files, or pg_execute_server_program).
  - Wire protocol (https://www.postgresql.org/docs/18/protocol-flow.html, section "COPY Operations"), quoted: "Copy-in mode (data transfer to the server) is initiated when the backend executes a COPY FROM STDIN SQL statement. The backend sends a CopyInResponse message to the frontend. The frontend should then send zero or more CopyData messages, forming a stream of input data. ... The frontend can terminate the copy-in mode by sending either a CopyDone message (allowing successful termination) or a CopyFail message" and "Copy-out mode ... The backend sends a CopyOutResponse message to the frontend, followed by zero or more CopyData messages (always one per row), followed by CopyDone."
  - tokio-postgres 0.7.18 API (https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html, crate updated 2026-06-12 per crates.io), quoted: "pub async fn copy_in<T, U>(&self, statement: &T) -> Result<CopyInSink<U>, Error> where T: ?Sized + ToStatement, U: Buf + 'static + Send" with "Executes a COPY FROM STDIN statement, returning a sink used to write the copy data. PostgreSQL does not support parameters in COPY statements, so this method does not take any. The copy must be explicitly completed via the Sink::close or finish methods. If it is not, the copy will be aborted." and "pub async fn copy_out<T>(&self, statement: &T) -> Result<CopyOutStream, Error>" with "Executes a COPY TO STDOUT statement, returning a stream of the resulting data." The binary_copy module provides BinaryCopyInWriter::new(sink, &[Type]) with write(), write_raw(), and finish() ("Completes the copy, returning the number of rows added."), and BinaryCopyOutStream::new(stream, &[Type]) yielding BinaryCopyOutRow with get()/try_get(). The upstream test file shows the usage (https://github.com/sfackler/rust-postgres/blob/master/tokio-postgres/tests/test/binary_copy.rs). The sync `postgres` crate (0.19.14, crates.io 2026-06-12) wraps these as CopyInWriter (std::io::Write) and CopyOutReader (std::io::Read).
  - Confidence: High.
- Finding 4.4: TRUNCATE, transactions, isolation, savepoints, prepared statements, cursors, LISTEN/NOTIFY.

```sql
TRUNCATE [ TABLE ] [ ONLY ] name [ * ] [, ... ] [ RESTART IDENTITY | CONTINUE IDENTITY ] [ CASCADE | RESTRICT ]
BEGIN [ WORK | TRANSACTION ] [ transaction_mode [, ...] ]
-- transaction_mode: ISOLATION LEVEL { SERIALIZABLE | REPEATABLE READ | READ COMMITTED | READ UNCOMMITTED } | READ WRITE | READ ONLY | [ NOT ] DEFERRABLE
SAVEPOINT savepoint_name;  ROLLBACK TO [ SAVEPOINT ] savepoint_name;  RELEASE [ SAVEPOINT ] savepoint_name;  COMMIT;
SET TRANSACTION transaction_mode [, ...]
SET SESSION CHARACTERISTICS AS TRANSACTION transaction_mode [, ...]
PREPARE name [ ( data_type [, ...] ) ] AS statement
DECLARE name [ BINARY ] [ ASENSITIVE | INSENSITIVE ] [ [ NO ] SCROLL ] CURSOR [ { WITH | WITHOUT } HOLD ] FOR query
FETCH [ direction ] [ FROM | IN ] cursor_name   -- NEXT | PRIOR | FIRST | LAST | ABSOLUTE count | RELATIVE count | count | ALL | FORWARD [count|ALL] | BACKWARD [count|ALL]
LISTEN channel;  NOTIFY channel [ , payload ];  UNLISTEN channel;
```

  - Quoted rules: "TRUNCATE is not MVCC-safe." and "TRUNCATE is transaction-safe with respect to the data in the tables: the truncation will be safely rolled back if the surrounding transaction does not commit." (sql-truncate); "DECLARE without WITH HOLD is useless outside a transaction block ... Therefore PostgreSQL reports an error if such a command is used outside a transaction block." (sql-declare); "By default (that is, when plan_cache_mode is set to auto), the server will automatically choose whether to use a generic or custom plan ... the first five executions are done with custom plans" (sql-prepare); "LISTEN takes effect at transaction commit." (sql-listen).
  - Confidence: High.
- Finding 4.5: EXPLAIN options in 18. https://www.postgresql.org/docs/18/sql-explain.html

```sql
EXPLAIN [ ( option [, ...] ) ] statement
-- option: ANALYZE [ boolean ] | VERBOSE [ boolean ] | COSTS [ boolean ] | SETTINGS [ boolean ] | GENERIC_PLAN [ boolean ]
--         | BUFFERS [ boolean ] | SERIALIZE [ { NONE | TEXT | BINARY } ] | WAL [ boolean ] | TIMING [ boolean ]
--         | SUMMARY [ boolean ] | MEMORY [ boolean ] | FORMAT { TEXT | XML | JSON | YAML }
```

  - Quoted: GENERIC_PLAN "Allow the statement to contain parameter placeholders like $1, and generate a generic plan that does not depend on the values of those parameters. ... This parameter cannot be used together with ANALYZE."; SERIALIZE "Serialization may only be enabled when ANALYZE is also enabled."; WAL "This parameter may only be used when ANALYZE is also enabled."; MEMORY "Include information on memory consumption by the query planning phase"; BUFFERS "Buffers information is automatically included when ANALYZE is used." (18 behavior). PostgreSQL 19 adds "IO [ boolean ]" (devel page). Safety note: EXPLAIN ANALYZE executes the statement, so the tool must classify the inner statement and wrap it in a transaction it rolls back when the mode is read-only.
  - Confidence: High.
- Finding 4.6: pg_stat_statements. https://www.postgresql.org/docs/18/pgstatstatements.html
  - Quoted: "The module must be loaded by adding pg_stat_statements to shared_preload_libraries in postgresql.conf, because it requires additional shared memory. This means that a server restart is needed to add or remove the module. In addition, query identifier calculation must be enabled in order for the module to be active, which is done automatically if compute_query_id is set to auto or on". The view has queryid, calls, total_exec_time, and (18) parallel-worker and wal_buffers_full columns. A tool can only read it when the extension is preloaded and created; it cannot enable it through SQL alone.
  - Confidence: High.

### Q5. Introspection: what psql runs, and the catalogs to use

- Finding 5.1: psql -E is the documented way to see the SQL, and describe.c is the authoritative text.
  - Quoted from https://www.postgresql.org/docs/18/app-psql.html: "-E --echo-hidden: Echo the actual queries generated by \d and other backslash commands. You can use this to study psql's internal operations. This is equivalent to setting the variable ECHO_HIDDEN to on." The queries below were read from https://raw.githubusercontent.com/postgres/postgres/REL_18_STABLE/src/bin/psql/describe.c (Authoritative, REL_18_STABLE, read 2026-09-12). relkind letters come from https://www.postgresql.org/docs/18/catalog-pg-class.html: "r = ordinary table, i = index, S = sequence, t = TOAST table, v = view, m = materialized view, c = composite type, f = foreign table, p = partitioned table, I = partitioned index".
  - Confidence: High.
- Finding 5.2: \d, \dt, \di, \dv, \dm, \ds and the + variant (listTables).

```sql
SELECT n.nspname as "Schema",
  c.relname as "Name",
  CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view' WHEN 'i' THEN 'index'
                 WHEN 'S' THEN 'sequence' WHEN 't' THEN 'TOAST table' WHEN 'f' THEN 'foreign table'
                 WHEN 'p' THEN 'partitioned table' WHEN 'I' THEN 'partitioned index' END as "Type",
  pg_catalog.pg_get_userbyid(c.relowner) as "Owner",
  c2.relname as "Table",                                                   -- only for \di
  CASE c.relpersistence WHEN 'p' THEN 'permanent' WHEN 't' THEN 'temporary' WHEN 'u' THEN 'unlogged' END as "Persistence",  -- + only
  am.amname as "Access method",                                            -- + only
  pg_catalog.pg_size_pretty(pg_catalog.pg_table_size(c.oid)) as "Size",   -- + only
  pg_catalog.obj_description(c.oid, 'pg_class') as "Description"          -- + only
FROM pg_catalog.pg_class c
     LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
     LEFT JOIN pg_catalog.pg_am am ON am.oid = c.relam
     LEFT JOIN pg_catalog.pg_index i ON i.indexrelid = c.oid              -- \di only
     LEFT JOIN pg_catalog.pg_class c2 ON i.indrelid = c2.oid              -- \di only
WHERE c.relkind IN ('r','p','')          -- \dt ; \di uses ('i','I') ; \dv ('v') ; \dm ('m') ; \ds ('S') ; \dE ('f') ; \d ('r','p','v','m','S','f','')
      AND n.nspname <> 'pg_catalog'
      AND n.nspname !~ '^pg_toast'
      AND n.nspname <> 'information_schema'
  AND pg_catalog.pg_table_is_visible(c.oid)   -- dropped when a pattern is given; S modifier removes the schema exclusions
ORDER BY 1,2;
```

- Finding 5.3: \d table_name (describeOneTableDetails) runs a sequence of queries. The ones a tool needs, with the table OID substituted for '%s':

```sql
-- 1. relation header
SELECT c.relchecks, c.relkind, c.relhasindex, c.relhasrules, c.relhastriggers, c.relrowsecurity, c.relforcerowsecurity,
       false AS relhasoids, c.relispartition,
       pg_catalog.array_to_string(c.reloptions || array(select 'toast.' || x from pg_catalog.unnest(tc.reloptions) x), ', '),
       c.reltablespace, CASE WHEN c.reloftype = 0 THEN '' ELSE c.reloftype::pg_catalog.regtype::pg_catalog.text END,
       c.relpersistence, c.relreplident, am.amname
FROM pg_catalog.pg_class c
 LEFT JOIN pg_catalog.pg_class tc ON (c.reltoastrelid = tc.oid)
 LEFT JOIN pg_catalog.pg_am am ON (c.relam = am.oid)
WHERE c.oid = '%s';

-- 2. columns
SELECT a.attname,
  pg_catalog.format_type(a.atttypid, a.atttypmod),
  (SELECT pg_catalog.pg_get_expr(d.adbin, d.adrelid, true)
   FROM pg_catalog.pg_attrdef d
   WHERE d.adrelid = a.attrelid AND d.adnum = a.attnum AND a.atthasdef),
  a.attnotnull,
  (SELECT c.collname FROM pg_catalog.pg_collation c, pg_catalog.pg_type t
   WHERE c.oid = a.attcollation AND t.oid = a.atttypid AND a.attcollation <> t.typcollation) AS attcollation,
  a.attidentity,
  a.attgenerated
FROM pg_catalog.pg_attribute a
WHERE a.attrelid = '%s' AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum;

-- 3. indexes (with the constraint that backs each)
SELECT c2.relname, i.indisprimary, i.indisunique, i.indisclustered, i.indisvalid,
  pg_catalog.pg_get_indexdef(i.indexrelid, 0, true),
  pg_catalog.pg_get_constraintdef(con.oid, true), contype, condeferrable, condeferred, i.indisreplident,
  c2.reltablespace, con.conperiod
FROM pg_catalog.pg_class c, pg_catalog.pg_class c2, pg_catalog.pg_index i
  LEFT JOIN pg_catalog.pg_constraint con ON (conrelid = i.indrelid AND conindid = i.indexrelid AND contype IN ('p','u','x'))
WHERE c.oid = '%s' AND c.oid = i.indrelid AND i.indexrelid = c2.oid
ORDER BY i.indisprimary DESC, c2.relname;

-- 4. check constraints
SELECT r.conname, pg_catalog.pg_get_constraintdef(r.oid, true)
FROM pg_catalog.pg_constraint r
WHERE r.conrelid = '%s' AND r.contype = 'c'
ORDER BY 1;

-- 5. foreign keys (non-partition case)
SELECT true as sametable, conname,
  pg_catalog.pg_get_constraintdef(r.oid, true) as condef,
  conrelid::pg_catalog.regclass AS ontable
FROM pg_catalog.pg_constraint r
WHERE r.conrelid = '%s' AND r.contype = 'f'
     AND conparentid = 0
ORDER BY conname;

-- 6. referenced by
SELECT conname, conrelid::pg_catalog.regclass AS ontable,
       pg_catalog.pg_get_constraintdef(oid, true) AS condef
  FROM pg_catalog.pg_constraint
 WHERE confrelid = %s AND contype = 'f'
ORDER BY conname;

-- 7. not-null constraints (18, verbose)
SELECT c.conname, a.attname, c.connoinherit, c.conislocal, c.coninhcount <> 0, c.convalidated
FROM pg_catalog.pg_constraint c JOIN pg_catalog.pg_attribute a ON (a.attrelid = c.conrelid AND a.attnum = c.conkey[1])
WHERE c.contype = 'n' AND c.conrelid = '%s'::pg_catalog.regclass
ORDER BY a.attnum;

-- 8. row-level security policies
SELECT pol.polname, pol.polpermissive,
  CASE WHEN pol.polroles = '{0}' THEN NULL ELSE pg_catalog.array_to_string(array(select rolname from pg_catalog.pg_roles where oid = any (pol.polroles) order by 1),',') END,
  pg_catalog.pg_get_expr(pol.polqual, pol.polrelid),
  pg_catalog.pg_get_expr(pol.polwithcheck, pol.polrelid),
  CASE pol.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' END AS cmd
FROM pg_catalog.pg_policy pol
WHERE pol.polrelid = '%s' ORDER BY 1;

-- 9. triggers
SELECT t.tgname, pg_catalog.pg_get_triggerdef(t.oid, true), t.tgenabled, t.tgisinternal,
  CASE WHEN t.tgparentid != 0 THEN
    (SELECT u.tgrelid::pg_catalog.regclass
     FROM pg_catalog.pg_trigger AS u, pg_catalog.pg_partition_ancestors(t.tgrelid) WITH ORDINALITY AS a(relid, depth)
     WHERE u.tgname = t.tgname AND u.tgrelid = a.relid AND u.tgparentid = 0
     ORDER BY a.depth LIMIT 1)
  END AS parent
FROM pg_catalog.pg_trigger t
WHERE t.tgrelid = '%s' AND (NOT t.tgisinternal OR (t.tgisinternal AND t.tgenabled = 'D'))
ORDER BY 1;

-- 10. rules
SELECT r.rulename, trim(trailing ';' from pg_catalog.pg_get_ruledef(r.oid, true)), ev_enabled
FROM pg_catalog.pg_rewrite r
WHERE r.ev_class = '%s' ORDER BY 1;
```

  - Source: describe.c on REL_18_STABLE (Authoritative). The pgsql-hackers thread https://www.postgresql.org/message-id/CAKAnmmJz8Hh%3D8Ru8jgzySPWmLBhnv4%3Doc_0KRiz-UORJ0Dex%2Bw%40mail.gmail.com (2023-12-11) confirms "\d mytable has the potential to run over a dozen SQL queries" and quotes the policy query verbatim.
  - Confidence: High.
- Finding 5.4: \dn, \du, \dp, \dx, \l, \df.

```sql
-- \dn (add n.nspacl and obj_description for \dn+)
SELECT n.nspname AS "Name", pg_catalog.pg_get_userbyid(n.nspowner) AS "Owner"
FROM pg_catalog.pg_namespace n
WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema'
ORDER BY 1;

-- \du (psql 16+ no longer lists memberof; use pg_auth_members for that)
SELECT r.rolname, r.rolsuper, r.rolinherit, r.rolcreaterole, r.rolcreatedb, r.rolcanlogin,
  r.rolconnlimit, r.rolvaliduntil,
  pg_catalog.shobj_description(r.oid, 'pg_authid') AS description,
  r.rolreplication, r.rolbypassrls
FROM pg_catalog.pg_roles r
WHERE r.rolname !~ '^pg_'
ORDER BY 1;

-- \dp
SELECT n.nspname as "Schema", c.relname as "Name",
  CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view' WHEN 'S' THEN 'sequence'
                 WHEN 'f' THEN 'foreign table' WHEN 'p' THEN 'partitioned table' END as "Type",
  c.relacl,
  pg_catalog.array_to_string(ARRAY(
    SELECT attname || E':\n  ' || pg_catalog.array_to_string(attacl, E'\n  ')
    FROM pg_catalog.pg_attribute a
    WHERE attrelid = c.oid AND NOT attisdropped AND attacl IS NOT NULL
  ), E'\n') AS "Column privileges"
  -- plus a policies array built from pg_catalog.pg_policy (see describe.c permissionsList)
FROM pg_catalog.pg_class c
     LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r','v','m','S','f','p')
      AND n.nspname <> 'pg_catalog' AND n.nspname <> 'information_schema'
  AND pg_catalog.pg_table_is_visible(c.oid)
ORDER BY 1, 2;

-- \dx
SELECT e.extname AS "Name", e.extversion AS "Version", ae.default_version AS "Default version",
       n.nspname AS "Schema", d.description AS "Description"
FROM pg_catalog.pg_extension e
  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace
  LEFT JOIN pg_catalog.pg_description d ON d.objoid = e.oid AND d.classoid = 'pg_catalog.pg_extension'::pg_catalog.regclass
  LEFT JOIN pg_catalog.pg_available_extensions() ae(name, default_version, comment) ON ae.name = e.extname
ORDER BY 1;

-- \l
SELECT d.datname as "Name", pg_catalog.pg_get_userbyid(d.datdba) as "Owner",
  pg_catalog.pg_encoding_to_char(d.encoding) as "Encoding",
  CASE d.datlocprovider WHEN 'b' THEN 'builtin' WHEN 'c' THEN 'libc' WHEN 'i' THEN 'icu' END AS "Locale Provider",
  d.datcollate as "Collate", d.datctype as "Ctype", d.datlocale as "Locale", d.daticurules as "ICU Rules",
  d.datacl,
  CASE WHEN pg_catalog.has_database_privilege(d.datname, 'CONNECT') OR pg_catalog.pg_has_role('pg_read_all_stats', 'USAGE')
       THEN pg_catalog.pg_size_pretty(pg_catalog.pg_database_size(d.datname)) ELSE 'No Access' END as "Size",  -- \l+
  t.spcname as "Tablespace", pg_catalog.shobj_description(d.oid, 'pg_database') as "Description"          -- \l+
FROM pg_catalog.pg_database d
  JOIN pg_catalog.pg_tablespace t on d.dattablespace = t.oid
ORDER BY 1;

-- \df
SELECT n.nspname as "Schema", p.proname as "Name",
  pg_catalog.pg_get_function_result(p.oid) as "Result data type",
  pg_catalog.pg_get_function_arguments(p.oid) as "Argument data types",
  CASE p.prokind WHEN 'a' THEN 'agg' WHEN 'w' THEN 'window' WHEN 'p' THEN 'proc' ELSE 'func' END as "Type"
FROM pg_catalog.pg_proc p
     LEFT JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
WHERE pg_catalog.pg_function_is_visible(p.oid)
      AND n.nspname <> 'pg_catalog' AND n.nspname <> 'information_schema'
ORDER BY 1, 2, 4;
```

  - Source: describe.c on REL_18_STABLE (Authoritative), cross-checked against echoed output on https://pgpedia.info/hacks/psql/show-internal-sql-commands-meta-data.html (Consensus, undated), https://github.com/postgres-ai/postgres-howtos/blob/main/0051_learn_about_schema_metadata_via_psql.md (Consensus), and https://dzone.com/articles/revealing-the-queries-behind-psqls-backslash-comma (Consensus, 2019-02-06, older \du form with memberof).
  - Confidence: High.
- Finding 5.5: information_schema versus pg_catalog.
  - Quoted from https://www.postgresql.org/docs/18/information-schema.html: "The information schema is defined in the SQL standard and can therefore be expected to be portable and remain stable, unlike the system catalogs, which are specific to PostgreSQL and are modeled after implementation concerns. The information schema views do not, however, contain information about PostgreSQL-specific features; to inquire about those you need to query the system catalogs or other PostgreSQL-specific views."
  - Practical rule for a DBA tool: use pg_catalog for everything (indexes, partitions, generated/identity columns, RLS, extensions, sizes, statistics have no information_schema equivalent; information_schema also filters by the caller's privileges, for example information_schema.columns "Only those columns are shown that the current user has access to"). Use information_schema.columns, .tables, .table_constraints, .key_column_usage only where portability to other engines matters. Consensus support: https://dbschema.com/blog/postgresql/reverse-engineering-a-postgresql-database ("information_schema has views for check constraints, table constraints and referential constraints, and not one that lists an index") and https://dba.stackexchange.com/questions/302587/what-is-faster-pg-catalog-or-information-schema.
  - Confidence: High.
- Finding 5.6: Key catalogs and views to read, with what each holds (from the PostgreSQL 18 catalog pages, Authoritative).
  - pg_namespace (schemas), pg_class (relations; relkind, relpersistence, reltuples, relpages, relhasindex, relrowsecurity, relreplident), pg_attribute (attnum, atttypid, attnotnull, attidentity, attgenerated, attisdropped, atthasdef), pg_attrdef (defaults and generation expressions via pg_get_expr(adbin, adrelid)), pg_index (indisprimary, indisunique, indisvalid, indkey; pg_get_indexdef), pg_constraint (contype in c, f, p, u, x, n, t; conenforced, convalidated, condeferrable, conperiod; pg_get_constraintdef), pg_trigger (pg_get_triggerdef), pg_proc (prokind, pg_get_functiondef), pg_type and pg_enum (enumlabel, enumsortorder), pg_roles (view over pg_authid without rolpassword) and pg_auth_members, pg_policy, pg_extension, pg_depend (dependency walk before DROP), pg_description (obj_description, col_description, shobj_description).
  - Statistics views: pg_stat_user_tables and pg_stat_user_indexes and pg_statio_user_tables (quoted: "The pg_stat_user_tables and pg_stat_sys_tables views contain the same information, but filtered to only show user and system tables respectively."), pg_stat_activity, pg_locks, pg_settings, pg_stat_database, pg_stat_replication, pg_stat_wal ("will always have a single row"), pg_stat_io ("one row for each combination of backend type, target I/O object, and I/O context"; added in 16 per https://github.com/postgres/postgres/commit/a9c70b46d and https://pgpedia.info/p/pg_stat_io.html), pg_stat_checkpointer ("will always have a single row"; added in 17 per the 17 release notes "Create system view pg_stat_checkpointer").
  - Size functions (https://www.postgresql.org/docs/18/functions-admin.html): pg_database_size(name|oid), pg_indexes_size(regclass), pg_relation_size(relation regclass [, fork text]) (quoted: "With one argument, this returns the size of the main data fork of the relation"), pg_table_size, pg_total_relation_size, pg_size_pretty, pg_size_bytes.

```sql
SELECT c.oid::regclass AS relation,
       pg_size_pretty(pg_total_relation_size(c.oid)) AS total,
       pg_size_pretty(pg_relation_size(c.oid))       AS heap,
       pg_size_pretty(pg_indexes_size(c.oid))        AS indexes,
       c.reltuples::bigint                            AS est_rows
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relkind IN ('r','p','m')
ORDER BY pg_total_relation_size(c.oid) DESC;
```

  - Confidence: High.
- Finding 5.7: Estimated versus exact row counts.
  - Quoted from https://www.postgresql.org/docs/18/catalog-pg-class.html: "reltuples float4: Number of live rows in the table. This is only an estimate used by the planner. It is updated by VACUUM, ANALYZE, and a few DDL commands such as CREATE INDEX. If the table has never yet been vacuumed or analyzed, reltuples contains -1 indicating that the row count is unknown." pg_stat_user_tables.n_live_tup and n_dead_tup are cumulative-statistics estimates. SELECT count(*) is exact and scans. A tool should show reltuples (treating -1 as unknown) by default and offer count(*) on demand with statement_timeout set. The 18.6 announcement adds a live warning: a parallel GIN build bug could leave reltuples "Infinity or NaN", and the fix is ANALYZE.
  - Confidence: High.
- Finding 5.8: Bloat estimation.
  - The PostgreSQL wiki page https://wiki.postgresql.org/wiki/Show_database_bloat (Authoritative wiki, last dated 2015-10-06 on the search index) says: "These queries is for informational purposes only. They provide a loose estimate of table growth activity only ... To obtain more accurate information about database bloat, please refer to the pgstattuple or pg_freespacemap contrib modules." It points to the ioguix queries. The ioguix repository https://github.com/ioguix/pgsql-bloat-estimation (Consensus, last commit 2022-08-23 "Fix index tuple header size", read raw 2026-09-12) is the standard statistical estimator; its README defines real_size, extra_size, extra_pct, fillfactor, bloat_size, bloat_pct, and "is_na: is the estimation "Not Applicable" ? If true, do not trust the stats." The table query, verbatim from table/table_bloat.sql:

```sql
/* WARNING: executed with a non-superuser role, the query inspect only tables and materialized view (9.3+) you are granted to read.
* This query is compatible with PostgreSQL 9.0 and more
*/
SELECT current_database(), schemaname, tblname, bs*tblpages AS real_size,
  (tblpages-est_tblpages)*bs AS extra_size,
  CASE WHEN tblpages > 0 AND tblpages - est_tblpages > 0
    THEN 100 * (tblpages - est_tblpages)/tblpages::float
    ELSE 0
  END AS extra_pct, fillfactor,
  CASE WHEN tblpages - est_tblpages_ff > 0
    THEN (tblpages-est_tblpages_ff)*bs
    ELSE 0
  END AS bloat_size,
  CASE WHEN tblpages > 0 AND tblpages - est_tblpages_ff > 0
    THEN 100 * (tblpages - est_tblpages_ff)/tblpages::float
    ELSE 0
  END AS bloat_pct, is_na
FROM (
  SELECT ceil( reltuples / ( (bs-page_hdr)/tpl_size ) ) + ceil( toasttuples / 4 ) AS est_tblpages,
    ceil( reltuples / ( (bs-page_hdr)*fillfactor/(tpl_size*100) ) ) + ceil( toasttuples / 4 ) AS est_tblpages_ff,
    tblpages, fillfactor, bs, tblid, schemaname, tblname, heappages, toastpages, is_na
  FROM (
    SELECT
      ( 4 + tpl_hdr_size + tpl_data_size + (2*ma)
        - CASE WHEN tpl_hdr_size%ma = 0 THEN ma ELSE tpl_hdr_size%ma END
        - CASE WHEN ceil(tpl_data_size)::int%ma = 0 THEN ma ELSE ceil(tpl_data_size)::int%ma END
      ) AS tpl_size, bs - page_hdr AS size_per_block, (heappages + toastpages) AS tblpages, heappages,
      toastpages, reltuples, toasttuples, bs, page_hdr, tblid, schemaname, tblname, fillfactor, is_na
    FROM (
      SELECT
        tbl.oid AS tblid, ns.nspname AS schemaname, tbl.relname AS tblname, tbl.reltuples,
        tbl.relpages AS heappages, coalesce(toast.relpages, 0) AS toastpages,
        coalesce(toast.reltuples, 0) AS toasttuples,
        coalesce(substring(
          array_to_string(tbl.reloptions, ' ')
          FROM 'fillfactor=([0-9]+)')::smallint, 100) AS fillfactor,
        current_setting('block_size')::numeric AS bs,
        CASE WHEN version()~'mingw32' OR version()~'64-bit|x86_64|ppc64|ia64|amd64' THEN 8 ELSE 4 END AS ma,
        24 AS page_hdr,
        23 + CASE WHEN MAX(coalesce(s.null_frac,0)) > 0 THEN ( 7 + count(s.attname) ) / 8 ELSE 0::int END
           + CASE WHEN bool_or(att.attname = 'oid' and att.attnum < 0) THEN 4 ELSE 0 END AS tpl_hdr_size,
        sum( (1-coalesce(s.null_frac, 0)) * coalesce(s.avg_width, 0) ) AS tpl_data_size,
        bool_or(att.atttypid = 'pg_catalog.name'::regtype)
          OR sum(CASE WHEN att.attnum > 0 THEN 1 ELSE 0 END) <> count(s.attname) AS is_na
      FROM pg_attribute AS att
        JOIN pg_class AS tbl ON att.attrelid = tbl.oid
        JOIN pg_namespace AS ns ON ns.oid = tbl.relnamespace
        LEFT JOIN pg_stats AS s ON s.schemaname=ns.nspname
          AND s.tablename = tbl.relname AND s.inherited=false AND s.attname=att.attname
        LEFT JOIN pg_class AS toast ON tbl.reltoastrelid = toast.oid
      WHERE NOT att.attisdropped
        AND tbl.relkind in ('r','m')
      GROUP BY 1,2,3,4,5,6,7,8,9,10
      ORDER BY 2,3
    ) AS s
  ) AS s2
) AS s3
ORDER BY schemaname, tblname;
```

  - The companion btree/btree_bloat.sql (100 lines, same repository) estimates index bloat and carries the same is_na caveat: "rows with is_na = 't' are known to have bad statistics ("name" type is not supported)". For exact numbers use the pgstattuple extension (pgstattuple(), pgstatindex()), which the wiki recommends and which the ioguix author uses as ground truth at https://blog.ioguix.net/postgresql/2014/09/10/Bloat-estimation-for-tables.html (Consensus, 2014-09-10). Note the query relies on pg_stats, so it is only as fresh as the last ANALYZE, and reltuples = -1 (never analyzed) makes it meaningless for that table.
  - Confidence: Medium (the queries are the community standard and unchanged since 2022; they are estimates by design, and no primary source certifies their accuracy on 18).

### Q6. Maintenance and operations

- Finding 6.1: VACUUM, ANALYZE, REINDEX, CLUSTER, CHECKPOINT syntax (18 pages, Authoritative; see synopses in Q3 for REINDEX).

```sql
VACUUM [ ( option [, ...] ) ] [ table_and_columns [, ...] ]
-- option: FULL | FREEZE | VERBOSE | ANALYZE | DISABLE_PAGE_SKIPPING | SKIP_LOCKED | INDEX_CLEANUP { AUTO | ON | OFF }
--         | PROCESS_MAIN | PROCESS_TOAST | TRUNCATE | PARALLEL integer | SKIP_DATABASE_STATS | ONLY_DATABASE_STATS | BUFFER_USAGE_LIMIT size
-- table_and_columns: [ ONLY ] table_name [ * ] [ ( column_name [, ...] ) ]
ANALYZE [ ( option [, ...] ) ] [ table_and_columns [, ...] ]   -- option: VERBOSE | SKIP_LOCKED | BUFFER_USAGE_LIMIT size
CLUSTER [ ( option [, ...] ) ] [ table_name [ USING index_name ] ]   -- option: VERBOSE
CHECKPOINT
```

  - Quoted: "VACUUM FULL rewrites the entire contents of the table into a new disk file with no extra space, allowing unused space to be returned to the operating system. This form is much slower and requires an ACCESS EXCLUSIVE lock on each table while it is being processed."; FREEZE "is equivalent to performing VACUUM with the vacuum_freeze_min_age and vacuum_freeze_table_age parameters set to zero. Aggressive freezing is always performed when the table is rewritten, so this option is redundant when FULL is specified."; PARALLEL "Perform index vacuum and index cleanup phases of VACUUM in parallel using integer background workers ... This option can't be used with the FULL option."; "Each backend running VACUUM without the FULL option will report its progress in the pg_stat_progress_vacuum view. Backends running VACUUM FULL will instead report their progress in the pg_stat_progress_cluster view."; VACUUM cannot run inside a transaction block. PostgreSQL 19 adds REPACK as the successor to VACUUM FULL and CLUSTER, with CONCURRENTLY.
  - Confidence: High.
- Finding 6.2: Autovacuum thresholds and how to read pg_stat_user_tables.
  - Quoted from https://www.postgresql.org/docs/18/routine-vacuuming.html: "vacuum threshold = Minimum(vacuum max threshold, vacuum base threshold + vacuum scale factor * number of tuples)" and "where the vacuum max threshold is autovacuum_vacuum_max_threshold, the vacuum base threshold is autovacuum_vacuum_threshold, the vacuum scale factor is autovacuum_vacuum_scale_factor, and the number of tuples is pg_class.reltuples." Defaults (as printed by pg_settings in a consensus source and matching the docs): autovacuum_vacuum_threshold 50, autovacuum_vacuum_scale_factor 0.2, autovacuum_vacuum_max_threshold 100000000 (new in 18), autovacuum_analyze_threshold 50, autovacuum_analyze_scale_factor 0.1, autovacuum_vacuum_insert_threshold 1000, autovacuum_vacuum_insert_scale_factor 0.2, autovacuum_naptime 60s, autovacuum_max_workers 3.
  - The pg_stat_all_tables columns to read: n_live_tup, n_dead_tup, n_mod_since_analyze, n_ins_since_vacuum, last_vacuum, last_autovacuum, last_analyze, last_autoanalyze, vacuum_count, autovacuum_count, analyze_count, autoanalyze_count, and in 18 total_vacuum_time, total_autovacuum_time, total_analyze_time, total_autoanalyze_time.

```sql
SELECT s.schemaname, s.relname,
       s.n_live_tup, s.n_dead_tup,
       round(100.0 * s.n_dead_tup / NULLIF(s.n_live_tup + s.n_dead_tup, 0), 2) AS dead_pct,
       s.n_mod_since_analyze,
       s.last_vacuum, s.last_autovacuum, s.last_analyze, s.last_autoanalyze,
       LEAST(current_setting('autovacuum_vacuum_max_threshold')::bigint,
             current_setting('autovacuum_vacuum_threshold')::bigint
             + current_setting('autovacuum_vacuum_scale_factor')::float * GREATEST(c.reltuples, 0)) AS vacuum_threshold,
       s.n_dead_tup > LEAST(current_setting('autovacuum_vacuum_max_threshold')::bigint,
             current_setting('autovacuum_vacuum_threshold')::bigint
             + current_setting('autovacuum_vacuum_scale_factor')::float * GREATEST(c.reltuples, 0)) AS needs_vacuum
FROM pg_stat_user_tables s
JOIN pg_class c ON c.oid = s.relid
WHERE s.schemaname = $1
ORDER BY s.n_dead_tup DESC;
```

  - Per-table overrides go through storage parameters: `ALTER TABLE t SET (autovacuum_vacuum_scale_factor = 0.01, autovacuum_vacuum_threshold = 1000);` (CREATE TABLE storage parameters page). Note that the formula above ignores per-table reloptions; a complete tool must read c.reloptions and prefer those values.
  - Confidence: High for the formula and columns; Medium for the exact default values, which I confirmed from a pg_settings dump in a consensus source (https://www.simplified.guide/postgresql/autovacuum-tune) rather than the docs page text.
- Finding 6.3: Checkpoints and WAL.
  - Quoted from https://www.postgresql.org/docs/18/wal-configuration.html: "Checkpoints are points in the sequence of transactions at which it is guaranteed that the heap and index data files have been updated with all information written before that checkpoint. At checkpoint time, all dirty data pages are flushed to disk and a special checkpoint record is written to the WAL file." CHECKPOINT is SQL, but it requires superuser or the pg_checkpoint predefined role. pg_stat_checkpointer (17+) and pg_stat_wal report activity; in 18 pg_stat_io gained WAL rows and track_wal_io_timing moved its timing there.
  - Confidence: High.
- Finding 6.4: pg_dump, pg_dumpall, pg_restore, pg_basebackup formats and flags (https://www.postgresql.org/docs/18/app-pgdump.html, app-pg-dumpall.html, app-pgrestore.html, app-pgbasebackup.html, backup-dump.html; Authoritative).
  - Quoted: pg_dump "-F format --format=format: p plain: Output a plain-text SQL script file (the default). c custom: Output a custom-format archive suitable for input into pg_restore ... This format is also compressed by default. d directory: ... This format is compressed by default using gzip and also supports parallel dumps. t tar: ... the tar format does not support compression."; "The most flexible output file formats are the "custom" format (-Fc) and the "directory" format (-Fd). They allow for selection and reordering of all archived items, support parallel restoration, and are compressed by default. The "directory" format is the only format that supports parallel dumps."; "Restoring a dump causes the destination to execute arbitrary code of the source superusers' choice."
  - pg_dumpall, quoted: "pg_dumpall is a utility for writing out ("dumping") all PostgreSQL databases of a cluster into one script file. ... pg_dumpall also dumps global objects that are common to all databases, namely database roles, tablespaces, and privilege grants for configuration parameters. (pg_dump does not save these objects.)" and "Cluster-wide data can be dumped alone using the pg_dumpall --globals-only option." Restore with "psql -X -f db.out -d postgres".
  - pg_restore, quoted: "If a database name is specified, pg_restore connects to that database and restores archive contents directly into the database. Otherwise, a script containing the SQL commands necessary to rebuild the database is created and written to a file or standard output." Parallel: "Only the custom and directory archive formats are supported with this option (-j) ... multiple jobs cannot be used together with the option --single-transaction." "pg_restore -l db.dump > db.list" lists the table of contents.
  - 18 additions: --statistics, --statistics-only, --no-statistics, --no-data, --no-schema, --no-policies, --sequence-data (release notes).
  - pg_basebackup, quoted: "Backups are always taken of the entire database cluster; it is not possible to back up individual databases or database objects. For selective backups, another tool such as pg_dump must be used." Formats "-F plain" and "-F tar"; "-X method --wal-method" with stream; "-R --write-recovery-conf: Creates a standby.signal file"; "incremental backup (--incremental) only works with server version 17 and later" and "An incremental backup cannot be used directly; instead, pg_combinebackup must first be used".
  - Confidence: High.
- Finding 6.5: PITR basics (https://www.postgresql.org/docs/18/continuous-archiving.html, Authoritative).
  - Quoted: "To enable WAL archiving, set the wal_level configuration parameter to replica or higher, archive_mode to on, specify the shell command to use in the archive_command configuration parameter or specify the library to use in the archive_library configuration parameter."; recovery: "Set recovery configuration settings in postgresql.conf ... and create a file recovery.signal in the cluster data directory."; "The one thing that you absolutely must specify is the restore_command"; "restore_command = 'cp /mnt/server/archivedir/%f %p'"; "You can specify the stop point, known as the "recovery target", either by date/time, named restore point or by completion of a specific transaction ID."; "The stop point must be after the ending time of the base backup".
  - Confidence: High.
- Finding 6.6: Logical replication setup (https://www.postgresql.org/docs/18/logical-replication-quick-setup.html and logical-replication-subscription.html, Authoritative).
  - Quoted: "wal_level = logical"; "CREATE PUBLICATION mypub FOR TABLE users, departments;"; "CREATE SUBSCRIPTION mysub CONNECTION 'dbname=foo host=bar user=repuser' PUBLICATION mypub;"; "The schema definitions are not replicated, and the published tables must exist on the subscriber. Only regular tables may be the target of replication."; "A published table must have a replica identity configured in order to be able to replicate UPDATE and DELETE operations". PostgreSQL 19 will add sequence replication and logical decoding without restart under wal_level = replica (19 notes).
  - Confidence: High.
- Finding 6.7: pg_upgrade (https://www.postgresql.org/docs/18/pgupgrade.html, Authoritative).
  - Quoted: "-c --check: check clusters only, don't change any data"; "-j njobs"; "-k --link: use hard links instead of copying files"; "--clone: Use efficient file cloning (also known as "reflinks" on some systems)"; "--copy: Copy files to the new cluster. This is the default."; "--swap: Move the data directories from the old cluster to the new cluster. Then, replace the catalog files with those generated for the new cluster. This mode can outperform --link, --clone, --copy, and --copy-file-range, especially on clusters with many relations." plus "it is recommended to use --sync-method=fsync with --swap."; "Always run the pg_upgrade binary of the new server, not the old one."; "Swap mode may be the fastest if there are many relations, but you will not be able to access your old cluster once the file transfer step begins."
  - Confidence: High.
- Finding 6.8: SQL-only versus host-binary split (derived from the pages above; High).
  - Through SQL over the connection: VACUUM (all options), ANALYZE, REINDEX (including CONCURRENTLY, outside a transaction), CLUSTER, CHECKPOINT (needs pg_checkpoint), REFRESH MATERIALIZED VIEW, all DDL, COPY TO STDOUT / FROM STDIN (logical export and import of one table or query, text, CSV, or binary), CREATE PUBLICATION / SUBSCRIPTION, reading every pg_stat view, pg_terminate_backend / pg_cancel_backend (needs pg_signal_backend), pg_switch_wal, pg_create_restore_point, ALTER SYSTEM (superuser or per-parameter grant), pg_reload_conf(), and pg_stat_reset().
  - Needs the CLI binaries on the host where they run, not SQL: pg_dump, pg_dumpall, pg_restore, pg_basebackup, pg_combinebackup, pg_upgrade, pg_createsubscriber, pg_checksums (offline), pg_rewind, initdb, and vacuumdb/reindexdb (which are just wrappers over SQL and can be replaced by the tool's own SQL). A tool that is "scoped to one database and one schema" can implement schema-level export with COPY TO STDOUT per table plus catalog-derived DDL, but a faithful full dump still needs pg_dump because pg_dump's dependency ordering and dump format are not reproducible from SQL alone.

### Q7. Safety mechanisms

- Finding 7.1: What READ ONLY blocks and what it does not.
  - Documented list, quoted from https://www.postgresql.org/docs/18/sql-set-transaction.html: "When a transaction is read-only, the following SQL commands are disallowed: INSERT, UPDATE, DELETE, MERGE, and COPY FROM if the table they would write to is not a temporary table; all CREATE, ALTER, and DROP commands; COMMENT, GRANT, REVOKE, TRUNCATE; and EXPLAIN ANALYZE and EXECUTE if the command they would execute is among those listed. This is a high-level notion of read-only that does not prevent all writes to disk." And from runtime-config-client: "default_transaction_read_only (boolean): A read-only SQL transaction cannot alter non-temporary tables. This parameter controls the default read-only status of each new transaction. The default is off (read/write)." and "transaction_read_only ... Any subsequent attempt to change it is equivalent to a SET TRANSACTION command."
  - Server source on REL_18_STABLE (Authoritative), read 2026-09-12:
    - src/backend/tcop/utility.c ClassifyUtilityCommandAsReadOnly returns COMMAND_IS_NOT_READ_ONLY for every T_Create*, T_Alter*, T_Drop*, T_GrantStmt, T_GrantRoleStmt, T_CommentStmt, T_IndexStmt, T_RefreshMatViewStmt, T_RenameStmt, T_RuleStmt, T_SecLabelStmt, T_TruncateStmt, T_ViewStmt, T_CreateTableAsStmt, T_CreatedbStmt, T_DropdbStmt with the comment "DDL is not read-only, and neither is TRUNCATE." T_CreateStmt is in that list, so CREATE TEMP TABLE is blocked too (the hot-standby page confirms: "Currently, temporary table creation is not allowed during read-only transactions").
    - Allowed in a read-only transaction: T_VariableSetStmt (SET), T_VariableShowStmt, T_PrepareStmt, T_ExecuteStmt, T_DeallocateStmt, T_DeclareCursorStmt, T_FetchStmt, T_ClosePortalStmt, T_DiscardStmt, T_LoadStmt, T_ListenStmt, T_NotifyStmt, T_UnlistenStmt, T_ExplainStmt, T_LockStmt, T_TransactionStmt, T_CheckPointStmt, T_CallStmt and T_DoStmt (with "Commands inside the DO block or the called procedure might not be read only, but they'll be checked separately"), T_VacuumStmt, T_ReindexStmt, T_ClusterStmt ("they don't change the database state in a way that would affect pg_dump output, so it's fine to run them in a read-only transaction"), T_CopyStmt when the target is temporary ("If the target table turns out to be non-temporary, DoCopy itself will call PreventCommandIfReadOnly"), and, surprisingly, T_AlterSystemStmt ("So, despite the fact that it writes to a file, it's read only!").
    - src/backend/commands/sequence.c calls PreventCommandIfReadOnly("nextval()") and PreventCommandIfReadOnly("setval()"), producing "cannot execute nextval() in a read-only transaction" (SQLSTATE 25006).
    - src/backend/executor/execMain.c ExecCheckXactReadOnly rejects any planned statement whose permission set on a non-temporary relation includes anything beyond SELECT, which covers INSERT, UPDATE, DELETE, MERGE, and SELECT ... FOR UPDATE/SHARE (row locking requires UPDATE privilege); temporary-namespace relations are exempt.
    - src/backend/commands/copy.c: "if (XactReadOnly && !rel->rd_islocaltemp) PreventCommandIfReadOnly("COPY FROM");".
  - What it does not stop, quoted from https://www.postgresql.org/docs/18/hot-standby.html: "In normal operation, "read-only" transactions are allowed to use LISTEN and NOTIFY" and "Note also that writes to remote databases using dblink module, and other operations outside the database using PL functions will still be possible, even though the transaction is read-only locally." Functions such as pg_terminate_backend(), pg_reload_conf(), pg_switch_wal(), lo_ functions with write intent, and untrusted-language functions are not caught by READ ONLY; role privileges are the only guard.
  - Recommended mechanics: for a read-only run, open every connection with `SET default_transaction_read_only = on` (or create the role with `ALTER ROLE ro SET default_transaction_read_only = on`) and still wrap each statement in `BEGIN READ ONLY ... ROLLBACK` for EXPLAIN ANALYZE. Do not rely on it alone; pair it with a role that has only SELECT and USAGE.
  - Confidence: High.
- Finding 7.2: Timeouts (https://www.postgresql.org/docs/18/runtime-config-client.html, Authoritative, quoted).
  - statement_timeout: "Abort any statement that takes more than the specified amount of time. ... A value of zero (the default) disables the timeout." and "In extended query protocol, the timeout starts running when any query-related message (Parse, Bind, Execute, Describe) arrives, and it is canceled by completion of an Execute or Sync message." and "Setting statement_timeout in postgresql.conf is not recommended because it would affect all sessions."
  - lock_timeout: "Abort any statement that waits longer than the specified amount of time while attempting to acquire a lock on a table, index, row, or other database object. The time limit applies separately to each lock acquisition attempt." and "if statement_timeout is nonzero, it is rather pointless to set lock_timeout to the same or larger value, since the statement timeout would always trigger first."
  - idle_in_transaction_session_timeout: "Terminate any session that has been idle (that is, waiting for a client query) within an open transaction for longer than the specified amount of time." and "an open transaction prevents vacuuming away recently-dead tuples ... so remaining idle for a long time can contribute to table bloat."
  - transaction_timeout (17+): "Terminate any session that spans longer than the specified amount of time in a transaction." and "If transaction_timeout is shorter or equal to idle_in_transaction_session_timeout or statement_timeout then the longer timeout is ignored."
  - Practical shape: `SET statement_timeout = '30s'; SET lock_timeout = '2s'; SET idle_in_transaction_session_timeout = '60s';` per session, and `SET LOCAL` inside DDL transactions that need longer limits.
  - Confidence: High.
- Finding 7.3: search_path pinning and CVE-2018-1058.
  - Quoted from https://wiki.postgresql.org/wiki/A_Guide_to_CVE-2018-1058%3A_Protect_Your_Search_Path: "The default value for search_path is $user,public"; "ALTER ROLE username SET search_path = "$user";"; "any user with the CREATEROLE permission have the ability to alter the default search_path for other users. If that is the case, then please use the "Do not allow users to create new objects in the public schema" strategy". The PostgreSQL 15 release note quotes: "Remove PUBLIC creation permission on the public schema (Noah Misch)" and "The change applies to new database clusters and to newly-created databases in existing clusters. Upgrading a cluster or restoring a database dump will preserve public's existing permissions." The documentation commit for the CVE (https://github.com/postgres/postgres/commit/5770172cb0c9df9e6ce27c507b449557e5b45124, Authoritative, 2018) states the rule for client tools: ""SET search_path = ..." and "SELECT pg_catalog.set_config(...)" are not vulnerable to such hijacking, so one can use either as the first command of a session." and "The principal defense for applications is "SELECT pg_catalog.set_config('search_path', '', false)", and the principal defense for databases is "REVOKE CREATE ON SCHEMA public FROM PUBLIC"."
  - For the tool: first statement on every connection `SELECT pg_catalog.set_config('search_path', '', false);` then always schema-qualify (or set search_path to exactly the run's schema plus pg_catalog); create functions with `SET search_path = <schema>, pg_temp`; run `REVOKE CREATE ON SCHEMA public FROM PUBLIC;` when the database predates 15.
  - Confidence: High.
- Finding 7.4: Identifier quoting and parameters.
  - Quoted from https://www.postgresql.org/docs/18/plpgsql-statements.html: "%I is equivalent to quote_ident, and %L is equivalent to quote_nullable. The format function can be used in conjunction with the USING clause". functions-string lists "quote_ident ( text ) → text" and "format ( formatstr text [, formatarg "any" [, ...] ] ) → text". Identifiers (schema, table, column, role names) can never be bound as parameters, so the tool must build them with an identifier-quoting routine equivalent to quote_ident (double quotes, doubled inner quotes) and pass all values as $n parameters. tokio-postgres binds values through `client.query("... WHERE id = $1", &[&id])` and "PostgreSQL does not support parameters in COPY statements". Server-side generation for dynamic SQL: `EXECUTE format('ALTER TABLE %I.%I ADD COLUMN %I %s', $1, $2, $3, $4)` where the type is validated against pg_type before interpolation.
  - Confidence: High.
- Finding 7.5: Role separation and the predefined roles (https://www.postgresql.org/docs/18/predefined-roles.html, Authoritative, quoted).
  - "pg_read_all_data allows reading all data (tables, views, sequences), as if having SELECT rights on those objects and USAGE rights on all schemas. This role does not bypass row-level security (RLS) policies."; "pg_write_all_data allows writing all data (tables, views, sequences), as if having INSERT, UPDATE, and DELETE rights"; "pg_monitor allows reading/executing various monitoring views and functions. This role is a member of pg_read_all_settings, pg_read_all_stats and pg_stat_scan_tables."; "pg_read_all_stats allows reading all pg_stat_* views"; "pg_signal_backend allows signaling another backend to cancel a query or terminate its session. Note that this role does not permit signaling backends owned by a superuser."; "pg_maintain allows executing VACUUM, ANALYZE, CLUSTER, REFRESH MATERIALIZED VIEW, REINDEX, and LOCK TABLE on all relations"; also pg_checkpoint, pg_create_subscription, pg_signal_autovacuum_worker (18), pg_read_server_files, pg_write_server_files, pg_execute_server_program, pg_use_reserved_connections, pg_database_owner.
  - Suggested role set for the tool's three modes:

```sql
-- read-only mode
CREATE ROLE dba_ro LOGIN NOINHERIT;
ALTER ROLE dba_ro SET default_transaction_read_only = on;
ALTER ROLE dba_ro SET search_path = '';
GRANT USAGE ON SCHEMA app TO dba_ro;
GRANT SELECT ON ALL TABLES IN SCHEMA app TO dba_ro;
ALTER DEFAULT PRIVILEGES IN SCHEMA app GRANT SELECT ON TABLES TO dba_ro;
GRANT pg_read_all_stats, pg_read_all_settings TO dba_ro;   -- or pg_monitor
-- write-only / read-write mode adds INSERT, UPDATE, DELETE (write-only is enforced by the tool's classifier, not by PostgreSQL, since UPDATE and DELETE need SELECT on referenced columns)
-- maintenance
GRANT pg_maintain, pg_signal_backend TO dba_rw;
```

  - Note: "write-only" is not a PostgreSQL privilege model. INSERT-only is possible (GRANT INSERT), but UPDATE and DELETE with a WHERE clause require SELECT on the referenced columns. Enforce write-only at the tool layer (refuse SelectStmt at top level) and keep the role privileges as small as the mode needs.
  - Confidence: High.
- Finding 7.6: Statement classification with libpg_query / pg_query.rs.
  - Facts: pg_query 6.2.0 on crates.io (updated 2026-08-03), CHANGELOG "6.2.0 2026-07-29: Upgrade to libpg_query 17-6.2.2; Add pg_query::summary function ...". libpg_query 18.0.0 was released 2026-05-21 ("Upgrade to Postgres 18"). pg_query.rs PR #79 "Upgrade to Postgres 18" is open (created 2026-07-16, updated 2026-08-06, not merged, checked with the GitHub API on 2026-09-12). pg_parse 0.15.0 (2026-06-18) is the lighter alternative from the same lineage. The parse result exposes `protobuf.stmts: Vec<RawStmt>` where each `RawStmt.stmt.node` is a `NodeEnum` variant; `NodeRef` mirrors it (https://docs.rs/pg_query/latest/pg_query/enum.NodeRef.html). `pg_query::split_with_parser` splits multi-statement strings; classify each piece and refuse the batch if any piece is disallowed.
  - Classification a tool can apply on the raw parse tree (node names from the NodeRef enum; behavior grounded in the server's own read-only classifier above):
    - Read (allowed in read-only mode): SelectStmt with no `into_clause`, no `locking_clause`, and no data-modifying statement inside `with_clause`; ExplainStmt whose inner statement is a read (EXPLAIN ANALYZE executes it, so wrap in BEGIN READ ONLY and ROLLBACK); VariableShowStmt; DeclareCursorStmt over a read, FetchStmt, ClosePortalStmt; PrepareStmt and ExecuteStmt classified by their inner statement; TransactionStmt (BEGIN, COMMIT, ROLLBACK, SAVEPOINT, RELEASE, ROLLBACK TO); CopyStmt with `is_from == false` and no filename or program (COPY ... TO STDOUT); ListenStmt, UnlistenStmt.
    - Write (read-write or write-only): InsertStmt, UpdateStmt, DeleteStmt, MergeStmt, CopyStmt with `is_from == true` to STDIN, SelectStmt with `into_clause` (SELECT INTO), SelectStmt with `locking_clause` (FOR UPDATE), SelectStmt whose CTE list contains InsertStmt, UpdateStmt, DeleteStmt, or MergeStmt, NotifyStmt, CallStmt and DoStmt (opaque; treat as write at minimum and refuse in read-only mode), VariableSetStmt for `role`, `session_authorization`, `search_path`, or `transaction_read_only` (allow only the tool's own settings).
    - DDL: CreateStmt, AlterTableStmt, DropStmt, IndexStmt, ViewStmt, CreateTableAsStmt, RefreshMatViewStmt, CreateSeqStmt, AlterSeqStmt, CreateFunctionStmt, AlterFunctionStmt, CreateTrigStmt, CreateEventTrigStmt, AlterEventTrigStmt, CreateEnumStmt, AlterEnumStmt, CompositeTypeStmt, CreateRangeStmt, CreateDomainStmt, AlterDomainStmt, AlterTypeStmt, CreateExtensionStmt, AlterExtensionStmt, CreateRoleStmt, AlterRoleStmt, AlterRoleSetStmt, DropRoleStmt, GrantStmt, GrantRoleStmt, AlterDefaultPrivilegesStmt, CreatePolicyStmt, AlterPolicyStmt, CommentStmt, SecLabelStmt, CreateTableSpaceStmt, DropTableSpaceStmt, CreatePublicationStmt, AlterPublicationStmt, CreateSubscriptionStmt, AlterSubscriptionStmt, DropSubscriptionStmt, CreateFdwStmt, CreateForeignServerStmt, CreateForeignTableStmt, ImportForeignSchemaStmt, CreateUserMappingStmt, DefineStmt (CREATE COLLATION, AGGREGATE, OPERATOR), RenameStmt, AlterOwnerStmt, AlterObjectSchemaStmt, CreateSchemaStmt, CreatedbStmt, AlterDatabaseStmt, DropdbStmt, RuleStmt, CreateStatsStmt, AlterSystemStmt.
    - Maintenance: VacuumStmt (`is_vacuumcmd` true for VACUUM, false for ANALYZE), ReindexStmt, ClusterStmt, CheckPointStmt, LockStmt.
    - Destructive, always requiring explicit confirmation or a mode flag: DropStmt (any `remove_type`), TruncateStmt, DropdbStmt, DropRoleStmt, DropOwnedStmt, DropSubscriptionStmt, DropTableSpaceStmt, AlterTableStmt whose `cmds` contain AlterTableCmd subtypes AT_DropColumn, AT_DropConstraint, AT_DetachPartition, AT_AlterColumnType with USING (rewrite), AT_SetUnLogged; DeleteStmt with `where_clause == None`; UpdateStmt with `where_clause == None`; DeleteStmt or UpdateStmt whose WHERE is a constant true (A_Const boolean) if the tool wants to close that loophole; COPY FROM to a file or PROGRAM; any statement outside the run's schema (compare RangeVar.schemaname against the scoped schema, and reject unqualified names when search_path is empty).
  - What parsing cannot see: function side effects (nextval, pg_terminate_backend, dblink, untrusted languages), trigger and rule side effects, EXPLAIN ANALYZE execution, and everything inside DO and CALL. Those are covered only by READ ONLY plus minimal role privileges.
  - Caveat: PostgreSQL 18-only syntax may fail in pg_query 6.2.0 (PG 17 grammar), for example `RETURNING WITH (OLD AS o, NEW AS n)`, `NOT ENFORCED`, `GENERATED ALWAYS AS (...) VIRTUAL`, `COPY ... (REJECT_LIMIT n)`. Options: vendor libpg_query 18.0.0 through the pg-18 branch of pg_query.rs (PR #79), fall back to sending the statement to the server under BEGIN READ ONLY and inspecting the error, or refuse anything the parser rejects.
  - Sources: https://crates.io/api/v1/crates/pg_query (registry, 2026-08-03); https://github.com/pganalyze/pg_query.rs/blob/main/CHANGELOG.md (Authoritative, 2026-07-29); https://api.github.com/repos/pganalyze/libpg_query/releases (2026-05-21); GitHub API pulls/79 (read 2026-09-12); https://docs.rs/pg_query/latest/pg_query/enum.NodeRef.html.
  - Confidence: High for versions and node names; Medium for the classification table itself (it is derived reasoning anchored on the server classifier, not a document that states it).

### Q8. Reference tool feature checklists

- Finding 8.1: pgAdmin 4 (https://www.pgadmin.org/docs/pgadmin4/latest/table_dialog.html and sibling dialog pages, Authoritative vendor docs, current 9.17 docs).
  - Table dialog tabs: "General, Columns, Constraints, Advanced, Parition, Parameter, and Security. The SQL tab displays the SQL code generated by dialog selections." Columns tab: name, data type, length/precision, scale, "Not NULL?" and "Primary key?" switches, default, collation, comment. Constraints tab: "Primary Key, Foreign Key, Check, Unique, Exclude" each with its own dialog. Partition tab: "Partitioned Table?", partition type (range, list, hash), partition keys, partitions panel with "Operation switch" to attach, From/To, In, Modulus/Remainder, and nested partitioned partitions. Foreign key dialog: General, Definition (Match type FULL/SIMPLE, deferrable, deferred, "Validated" switch), Columns, Action (On update, On delete: NO ACTION, RESTRICT, CASCADE, SET NULL, SET DEFAULT). Object dialogs also exist for Column, Index, Primary key, Unique, Exclusion constraint, Check, RLS Policy, Rule, Trigger, Compound Trigger, Foreign Table, plus "Management Basics, Backup and Restore, Developer Tools (Query Tool, View/Edit Data), Processes".
  - Confidence: High.
- Finding 8.2: DBeaver (https://dbeaver.com/docs/dbeaver/Database-driver-PostgreSQL/, Data-Editor, Creating-columns, Implementing-Constraints; vendor docs, undated pages read 2026-09-12).
  - Objects listed for PostgreSQL: "Databases, Schemas, Data types, Tables, Columns, Constraints, Indexes, Foreign Keys, Dependencies, References, Partitions, Triggers, Rules, Policies, Foreign Tables, Views, Materialized Views, Functions, Sequences, Data types, Aggregate functions, Event Triggers, Extensions, Storage, Tablespaces, Roles, Administer, Jobs, Session Manager, Lock Manager". PostgreSQL-specific features: "PostgreSQL Arrays, PostgreSQL Structures, PostgreSQL Extensions, PostgreSQL Permissions, PostgreSQL Policies, PostgreSQL Roles, PostgreSQL Partitions, PostgreSQL Dependencies, PostgreSQL Tools, PostgreSQL Foreign Tables" plus "Data Import, Data Export, Session Manager, Lock Manager, How to Backup/Restore data, Execute script with local client, Schema Compare, GIS Guide, ERD Guide". Column editor fields: "Name, Data type, Identity, Collation, Not null, Default, Comment, Unique, Type (Primary Key or Unique Key), Name". Constraint editor: "Type: Primary Key, Unique Key, Check; Columns". Data editor: grid, text, JSON, record views, filters, ordering, "Calculate total row count", and "To be able to save column value changes, a table must have some unique key (primary key or unique index)", with virtual keys as a fallback; read-only connections block edits.
  - Confidence: Medium (pages carry no publication date).
- Finding 8.3: DataGrip (https://www.jetbrains.com/help/datagrip/create-and-modify-dialogs.html 2026-03-23, working-with-the-data-editor.html 2026-07-06, foreign-keys.html, indexes.html, database-explorer.html; vendor docs).
  - Create/Modify dialogs cover "schemas, tables, columns, keys, foreign keys, indexes, checks, virtual columns, virtual foreign keys, views, users and roles, and virtual views"; every dialog has a preview pane: "The preview pane on the lower part of the dialog shows the SQL script that DataGrip will run". Column fields: "Name, Comment, Data Type, Not Null, Default Expression"; for PostgreSQL "you can add and edit column check constraints" since 2022.1. Foreign key fields: Target Table, Columns, "Deferrable", "Initially Deferred", "On Delete", "On Update". Index fields: "Unique", Columns with "Order" and "Collation". Data editor: Table, Tree, Text, Transpose modes; "Preview Pending Changes"; value editor for arrays and JSON; export/import; "Export with 'pg_dump'…" shells out to the native binary ("They are not integrated into DataGrip"). Explorer icons list Access Method, Aggregate, Check, Collation, Extension, Foreign Data Wrapper, Foreign Table, Materialized View, Operator, Policy (via rule/other), Role, Routine, Rule, Sequence, Server, Tablespace, Trigger, User Mapping, View.
  - Confidence: High for the dated pages.
- Finding 8.4: TablePlus (https://docs.tableplus.com/gui-tools/working-with-table/table, column, index, constraint, trigger; vendor docs dated 2019-10-31 in the search index).
  - Table structure view (Cmd+Ctrl+]) with inline editing and Cmd+S commit; "Add new column by clicking on the + Column ... Specify the column's attributes: name, datatype, nullability, default"; indexes via "+ Index" and inline edit or delete; constraints: "NOT NULL Constraint" via is_nullable, "PRIMARY KEY Constraint" via the Primary Key box, "FOREIGN KEY constraint" via the foreign_key field popup, "DEFAULT Constraint" via column_default; triggers button in the structure view; "Definition" button to show CREATE TABLE; data editing inline with pending changes committed on Cmd+S; filters with column, condition, value; 300 rows per page by default; SQL query editor; export table.
  - Confidence: Medium (vendor docs are current pages but dated 2019; treat the checklist as a floor, not the current full feature set).
- Finding 8.5: psql (https://www.postgresql.org/docs/18/app-psql.html, Authoritative).
  - Backslash coverage for the checklist: \l databases, \dn schemas, \d \dt \di \dv \dm \ds \dE relations with S and + modifiers ("If + is appended to the command name, each object is listed with its persistence status (permanent, temporary, or unlogged), physical size on disk, and associated description"), \d name for columns, indexes, constraints, policies, triggers, rules, publications, \df functions, \dT types, \dD domains, \dx extensions, \du roles, \dp privileges, \ddp default privileges, \dRp \dRs publications and subscriptions, \dP partitioned relations, \dX extended statistics, \db tablespaces, \dc conversions, \dO collations, \dew foreign data wrappers, \des servers, \det foreign tables, \dy event triggers, \sf function source, \sv view source, \copy client-side COPY, \watch, \gexec, \if. PostgreSQL 19 adds %S (search path) and %i (hot standby) prompt escapes and comment display for \dRp+, \dRs+, \dX+.
  - Confidence: High.

## 3. Conflicts found and how they were resolved

- Conflict A: PostgreSQL 19 release notes list "Allow partitions to be merged and split using ALTER TABLE ... MERGE/SPLIT PARTITIONS" (https://www.postgresql.org/docs/19/release-19.html, AS OF 2026-07-18, read 2026-09-12), but the feature was reverted. Resolution: the pgsql-committers message https://www.postgresql.org/message-id/E1wzVGA-00000002JYO-2zaG%40gemulon.postgresql.org (2026-08-27) says "Revert support for ALTER TABLE ... MERGE/SPLIT PARTITION(S) commands ... The feature is reverted due to multiple design issues which are too late to address in this release cycle." with "Branch: REL_19_STABLE" and commit 3e8bcc8644feaa9ca1cc954197b6994817af4290, confirmed on https://git.postgresql.org/gitweb/?p=postgresql.git;a=commit;h=3e8bcc8644feaa9ca1cc954197b6994817af4290 (committer date Thu, 27 Aug 2026 07:03:53 +0000). Dimitri Fontaine's post https://tapoueh.org/blog/2026/09/getting-ready-for-postgresql-19/ (Consensus, 2026-09-03) explains the notes lag: "the entry is still in the release-note source on the release branch." Authority and recency favor the commit. Do not plan for MERGE/SPLIT PARTITIONS in 19. It was also pulled from 17 in August 2024 (commit 3890d90c15, CVE-2014-0062 concerns).
- Conflict B: Beta page snapshot in the Exa index said "The current beta release is PostgreSQL 19 Beta 1" while the live page says Beta 3. Resolution: live fetch on 2026-09-12 wins (Beta 3).
- Conflict C: GA date wording. The roadmap says "planned for September 2026"; the beta announcements say "around September/October 2026"; the open items wiki says "GA: TBD" and "RC 1: TBD" with Beta 4 on 2026-09-24. Resolution: all three are official and not contradictory; the precise reading is that no GA date exists yet, Beta 4 comes first, and the window is September or October 2026.
- Conflict D: NOT ENFORCED for CHECK constraints via ALTER CONSTRAINT. An EDB blog (2026-03-27) says altering enforceability applies only to foreign keys in 18, while the 18 ALTER TABLE grammar shows ENFORCED | NOT ENFORCED on ALTER CONSTRAINT without restriction. Resolution: the 19 release notes state "Allow ALTER TABLE ALTER CONSTRAINT ... [NOT] ENFORCED for CHECK constraints ... Previously enforcement changes were only supported for foreign key constraints", so in 18 the ALTER CONSTRAINT enforceability switch works for foreign keys only, while CHECK constraints can be created NOT ENFORCED. Authoritative notes win.
- Conflict E: \du output. Older consensus articles (2019) show a "memberof" column in the \du query; describe.c on REL_18_STABLE has no such column. Resolution: primary source wins; membership must be read from pg_auth_members.
- Conflict F: pg_query parser version. Several search snippets imply pg_query.rs supports PostgreSQL 18 (the libpg_query README lists 18 as "Active development", and a Zig binding wraps 18.4). Resolution: the pg_query.rs CHANGELOG (6.2.0 uses libpg_query 17-6.2.2) and the open PR #79 are the primary evidence; the crate published to crates.io is on the 17 grammar as of 2026-09-12.
- Conflict G: 18.5. The release notes index lists 18.6, 18.4, 18.3, 18.2, 18.1, 18.0. Resolution: the 18.6 notes and the announcement both say 18.5 was never released; no conflict after reading them.

## 4. Open questions

- What is the PostgreSQL 19 GA date? Searched postgresql.org roadmap, beta page, all three beta announcements, the open items wiki, and Serper, Exa, Tavily, Brave, and WebSearch for "PostgreSQL 19 GA date" and "RC1". The project has not announced RC1 or GA; only Beta 4 (2026-09-24) is scheduled.
- When will pg_query.rs publish a crate built on libpg_query 18.0.0? PR #79 has been open since 2026-07-16 with the last update on 2026-08-06; no release date is stated anywhere I searched.
- Do the ioguix bloat estimators stay accurate on PostgreSQL 18 heap pages? The repository's last commit is 2022-08-23 and no source evaluates it against 18; the page and tuple header sizes it assumes have not changed, but I found no test evidence. Treat it as an estimate and offer pgstattuple for exact figures.

## 5. Methodology

- Searches run per tool (whole brief, not per sub-question):
  - Exa (mcp__exa__web_search_exa): 30
  - Serper (mcp__serper__google_search): 30
  - Exa and Serper gap: 0 (equal)
  - Tavily (mcp__tavily__tavily_search): 6
  - Brave (mcp__brave-search__brave_web_search): 6
  - Built-in WebSearch: 6
  - Serper specialized indexes (scholar, news, patents): 0; no query needed them.
- Fetches:
  - Local reader (mcp__read-website__read_website): 10 calls (versioning, release index, roadmap, 18.6 news, PG 19 open items, docs/19/release-19, docs/release/18.6, developer/beta, docs/18/release-18, docs/17/release-17). One came back thin (docs/18/release-18.html returned only the monitoring fragment) and was judged failed on content.
  - Paid JavaScript-capable fetch: 1 (mcp__tavily__tavily_extract on docs/18/release-18.html, which returned the full 80 KB page).
  - Direct HTTP fetches through curl in Bash (free): 84 PostgreSQL 18 reference and chapter pages (synopsis blocks extracted from `<pre class="synopsis">`), docs/19/release-19.html and docs/18/index.html headers, src/bin/psql/describe.c, src/backend/tcop/utility.c, src/backend/commands/sequence.c, src/backend/executor/execMain.c, src/backend/commands/copy.c (all REL_18_STABLE from raw.githubusercontent.com), ioguix table_bloat.sql and btree_bloat.sql, pg_query.rs CHANGELOG.md, the git.postgresql.org commit page and the postgresql.org message-id page for the revert.
  - Registry and repository APIs: crates.io API for tokio-postgres (0.7.18), postgres (0.19.14), pg_query (6.2.0), pg_parse (0.15.0); GitHub API for libpg_query releases and tags (18.0.0 on 2026-05-21; 17-6.2.3 tag), pg_query.rs tags (v6.2.0), rust-postgres tags; `gh api` (authenticated, after the anonymous limit was hit) for pg_query.rs PR #79 state, open PRs, rust-postgres latest release (postgres-v0.19.14, 2026-06-12), and the ioguix last commit (2022-08-23).
- Primary sources read in full or in the relevant section (all as of 2026-09-12):
  - https://www.postgresql.org/support/versioning/
  - https://www.postgresql.org/docs/release/
  - https://www.postgresql.org/developer/roadmap/
  - https://www.postgresql.org/developer/beta/
  - https://www.postgresql.org/about/news/postgresql-186-1711-1615-1519-1424-and-19-beta-3-released-3365/
  - https://wiki.postgresql.org/wiki/PostgreSQL_19_Open_Items
  - https://www.postgresql.org/docs/19/release-19.html
  - https://www.postgresql.org/docs/release/18.6/
  - https://www.postgresql.org/docs/18/release-18.html
  - https://www.postgresql.org/docs/17/release-17.html
  - https://www.postgresql.org/message-id/E1wzVGA-00000002JYO-2zaG%40gemulon.postgresql.org and the matching gitweb commit page
  - 84 PostgreSQL 18 documentation pages listed in the scratchpad (sql-createtable through sql-lock, catalogs, monitoring-stats, runtime-config-client, routine-vacuuming, functions-admin, functions-string, plpgsql-statements, ddl-schemas, ddl-generated-columns, ddl-rowsecurity, information-schema, app-psql, wal-configuration, pgstatstatements, pgupgrade family via search highlights)
  - REL_18_STABLE source: describe.c, utility.c, sequence.c, execMain.c, copy.c
  - https://wiki.postgresql.org/wiki/A_Guide_to_CVE-2018-1058%3A_Protect_Your_Search_Path and https://github.com/postgres/postgres/commit/5770172cb0c9df9e6ce27c507b449557e5b45124
  - https://github.com/ioguix/pgsql-bloat-estimation (README, table_bloat.sql, btree_bloat.sql) and https://wiki.postgresql.org/wiki/Show_database_bloat
  - docs.rs tokio-postgres Client and binary_copy pages; rust-postgres binary_copy.rs test file
  - pgAdmin 4, DBeaver, DataGrip, TablePlus documentation pages named in Q8
- Sources evaluated versus selected: about 140 distinct URLs surfaced across the tools; 61 selected for citations; the rest were leads or duplicates.
- Sources discarded as false positives or low trust, with reason:
  - https://deepwiki.com/rust-postgres/rust-postgres/11.1-copy-operations: AI-generated wiki; replaced by docs.rs and the upstream source.
  - https://runebook.dev/... and https://adhdecode.com/...: content-farm summaries of the docs; replaced by the docs.
  - https://www.tenable.com/u?521c69a9=: mirror of the PostgreSQL wiki page; original used instead.
  - https://migrationpilot.dev/... (ALTER TYPE rules): vendor marketing pages; claims were verified against the ALTER TYPE and enum docs and the docs are cited.
  - https://www.matthewswong.com/..., https://bishrulhaq.com/..., https://www.bbarroso.page/..., https://neon.com/postgresql/18-new-features, https://www.bytebase.com/blog/...: secondary summaries of the 18 release; used only as leads; every claim was taken from the release notes.
  - Exa's cached snapshot of https://www.postgresql.org/developer/beta/ ("Beta 1"): stale index copy; live page used.
  - dzone.com (2019) \du query with memberof: outdated for 16+; describe.c used.
  - pgexperts pgx_scripts table_bloat_check.sql: valid alternative but older (requires 8.4+) and not read in full; ioguix cited instead.
  - Reddit thread on introspection (Tavily result): experience report only; not cited.
- Tools marked dead: none. Anonymous GitHub REST hit its 60-request limit (HTTP 403) once and was replaced by the authenticated `gh` CLI; this was not a tool outage under Article VII. Six Serper queries with `site:postgresql.org/docs/18` path scoping returned zero organic results; rephrasing to `site:postgresql.org` worked, so Serper was never retried as dead.
- Depth honored: exhaustive for the eight numbered questions, with every version, date, and maintenance claim read from postgresql.org pages, the crates.io API, the GitHub API, or the server source, not from snippets.

## 6. Limitations

- The PostgreSQL 19 content is a beta snapshot. The notes on docs/19 are marked "AS OF 2026-07-18" and "Release date: 2026-??-??"; at least one listed feature (MERGE/SPLIT PARTITIONS) is already gone and GROUP BY ALL was reverted before Beta 3. Recheck the 19 notes after Beta 4 (2026-09-24) and at RC1.
- The read-only classification in Q7 mixes documented text with facts read from the REL_18_STABLE source (utility.c, sequence.c, execMain.c, copy.c). Those files can change in later minors; the behavior described matches 18.6.
- The statement classification map for libpg_query (Finding 7.6) is my synthesis of the server's own read-only classifier and the NodeRef enum; no single document states it. Test it against the tool's grammar coverage, especially the PG 18 syntax that pg_query 6.2.0 cannot parse.
- The bloat estimator is a statistical approximation whose repository was last changed in 2022; use pgstattuple for exact figures.
- TablePlus documentation pages are dated 2019-10-31 and DBeaver pages carry no dates; their checklists may understate current features.
- Autovacuum default values were confirmed from a pg_settings listing in a consensus source, not from the docs page text itself; the formula and column names are from the docs.
- No page fetched during this run contained instructions aimed at the agent.
- No SQL was executed against any database, and no credential file was opened.
