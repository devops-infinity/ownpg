# ADR-0018: PostgreSQL 14 through 18 are supported with feature gates, and 19 is best effort until GA

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

As of 2026-09-12 the supported PostgreSQL majors are 18.6, 17.11, 16.15, 15.19, and 14.24; 13 is end-of-life; 14 ends on 2026-11-12; 19 is at Beta 3 with Beta 4 due 2026-09-24 and no GA date, and at least one listed 19 feature (partition merge and split) has already been reverted. The DBA surface differs by version: `pg_stat_io` (16), `MERGE RETURNING`, `transaction_timeout`, `pg_stat_checkpointer`, the `MAINTAIN` privilege (17), `RETURNING OLD` and `NEW`, `NOT ENFORCED`, virtual generated columns, named `NOT NULL` constraints, `uuidv7()`, `autovacuum_vacuum_max_threshold` (18). `pg_query` 6.2.0 parses with the 17 grammar. Which versions does OwnPG support, and how?

## Decision Drivers

- The Principal Architect runs 18; teams run 14 through 17 in the field.
- Every catalog query and generated statement must be correct on every supported version.
- No 19-only syntax at launch, but nothing that breaks on 19 either.
- The support window should follow the PostgreSQL project's own.

## Options Considered

### Option A: 14 through 18 with gates on `server_version_num`, 19 best effort

Read `server_version_num` at connect; expose a `features` map in `doctor` and in `server/discover` instructions; gate tools and columns (for example the top-queries tool checks the `pg_stat_statements` version, the health tool adds `pg_stat_io` on 16 and later, the update tool offers `returning_old_new` only on 18). Run the integration suite in CI against the `postgres` images for 14, 15, 16, 17, and 18, and a separate scheduled workflow against the 19 beta image that is not a required check. Drop 14 when the project drops it (2026-11-12) in the next minor release after that date, following ADR-0019.

- Pros: matches the project's support window; every tested version is a CI matrix row.
- Pros: 19 surprises show up on the schedule without blocking releases.
- Cons: five images in CI; version branches in the catalog queries.

### Option B: 18 only

- Pros: one code path, the newest features everywhere.
- Cons: excludes most deployed servers; `pg_query` cannot even parse some 18 syntax yet.

### Option C: 12 through 18

- Pros: covers old installs.
- Cons: 12 and 13 are end-of-life; maintaining them contradicts the project's own policy.

### Option D: Do nothing (untested version claims)

- Cons: catalog queries would break silently on older servers.

## Decision

We will support PostgreSQL 14 through 18 with feature gates keyed on `server_version_num`, test each in CI, treat 19 as best effort until its GA, and drop a major in the first minor release after the PostgreSQL project ends its support.

Option A won because it mirrors the upstream support window and turns version differences into tested branches.

## Consequences

- Positive: the same binary works on a laptop's 18 and a server's 14.
- Positive: `doctor` tells the user which features their server offers.
- Negative: five CI images and a scheduled 19 job.
- Neutral: 19 features on the watch list (`REPACK`, `FOR PORTION OF`, `pg_get_role_ddl()`, `pg_stat_lock`, `COPY TO` JSON, `ON CONFLICT DO SELECT`, `lz4` TOAST default, JIT off by default, `standard_conforming_strings` forced on) are re-evaluated at GA.

## Reversibility

Cheap to reverse: adding or dropping a version is a CI matrix row and a gate.

## Sources

- https://www.postgresql.org/support/versioning/ (read 2026-09-12)
- https://www.postgresql.org/docs/18/release-18.html and https://www.postgresql.org/docs/17/release-17.html
- https://wiki.postgresql.org/wiki/PostgreSQL_19_Open_Items and https://www.postgresql.org/message-id/E1wzVGA-00000002JYO-2zaG%40gemulon.postgresql.org (2026-08-27 revert)
- https://github.com/pganalyze/pg_query.rs/blob/main/CHANGELOG.md
- `research/05-postgresql-18-dba-surface.md`, questions 1 and 2
