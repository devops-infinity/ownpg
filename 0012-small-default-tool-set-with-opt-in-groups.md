# ADR-0012: A small default tool set, opt-in tool groups, and a token budget checked in CI

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

Token cost is the top complaint about database MCP servers: DBHub ships 2 tools at 1.4k tokens, the Google Toolbox 28 tools at 19.0k, Supabase 19.3k; one Rust server ships 133 tools behind category flags. Claude's tool choice degrades past 30 to 50 tools, Claude Code defers tool schemas behind tool search, and Anthropic's review wants tool names under 64 characters and split write tools rather than a catch-all. OwnPG must cover the whole lifecycle (PRD FR-17 through FR-26) without loading all of it into every conversation. How is the surface shaped?

## Decision Drivers

- The default conversation pays for exploration and reads only.
- Every group is discoverable by name and stays under a measured budget.
- Names follow the review rules and tool search works on them.
- Structured output with `outputSchema` on every tool that returns data.

## Options Considered

### Option A: Default set plus opt-in groups, meta-tools with an `operation` enum inside each group

Default (loaded in every mode): `pg_list_objects` (schemas, tables, views, sequences, functions with `detail_level`), `pg_describe` (one object, full detail), `pg_run_query` (read statement with caps and a cursor handle; the name avoids a clash with the `pg_query` crate), `pg_count`, `pg_explain`, `pg_health`, `pg_doctor`. Groups beyond the default: `write` and `transactions` load automatically in `write-only` and `read-write` mode; `ddl`, `roles`, `maintenance`, `monitoring`, and `host` load only when named in `--tools` or the profile, and only in a mode that allows them: `ddl` (`pg_table`, `pg_column`, `pg_constraint`, `pg_index`, `pg_view`, `pg_sequence`, `pg_routine`, `pg_trigger`, `pg_type`, `pg_extension`, `pg_comment`, each with an `operation` enum such as create, alter, drop, rename), `write` (`pg_insert`, `pg_update`, `pg_delete`, `pg_merge`, `pg_run_write`, `pg_copy`), `roles` (`pg_role`, `pg_grant`, `pg_policy`, `pg_privileges`), `maintenance` (`pg_vacuum`, `pg_analyze`, `pg_reindex`, `pg_refresh`, `pg_vacuum_needs`, `pg_backend`), `monitoring` (`pg_activity`, `pg_locks`, `pg_replication`, `pg_wal`, `pg_indexes_health`, `pg_bloat`, `pg_settings`, `pg_top_queries`), `transactions` (`pg_transaction`), `host` (`pg_dump`, `pg_dumpall_globals`, `pg_restore`, `pg_basebackup`, `pg_upgrade_check`). Names use the `pg_` prefix, verb or noun plus resource, all under 64 characters. Descriptions read like a docstring for a new hire and never instruct the model how to behave. Every data-returning tool declares an `outputSchema`, returns `structuredContent` plus compact text, and states its caps. CI measures `tools/list` for the default set and fails above 6,000 tokens. One registry table in `groups.rs` lists every tool with its group, allowed modes, remote scope (`ownpg:read` for the default set and `monitoring`, `ownpg:write` for `write` and `transactions`, and one scope per remaining group), and annotations; a test asserts every tool appears once.

- Pros: the default conversation costs about what DBHub's does, with more coverage.
- Pros: groups map onto access modes (write, transactions, ddl, roles, and maintenance never load in read-only).
- Cons: the `operation` enum needs conditional validation per operation in the schema and the handler.

### Option B: One tool per operation (about 80 tools)

- Pros: the simplest schemas.
- Cons: 19k plus tokens, tool choice degrades, and clients cap tool counts (Windsurf at 100).

### Option C: One `execute_sql` tool plus one `describe` tool

- Pros: 1.4k tokens.
- Cons: the model writes every DDL by hand; no dry-run rendering, no per-operation annotations, no operation-level confirmation; the review rejects catch-all tools.

### Option D: Do nothing (grow tools as needed)

- Cons: the budget would creep past 30 tools within a milestone.

## Decision

We will ship a seven-tool default set and opt-in groups of grouped tools with an `operation` enum, name them with a `pg_` prefix under 64 characters, declare an `outputSchema` on every data tool, and fail CI when the default set exceeds 6,000 tokens.

Option A won because it keeps the default conversation cheap while leaving the whole lifecycle one flag away.

## Consequences

- Positive: read-only users load seven tools; DBA users load the groups they need.
- Positive: tool search in Claude Code finds `pg_index` for "add an index" because the name says so.
- Negative: grouped tools carry larger schemas; each group's cost is published in the README.
- Neutral: resources and prompts (PRD FR-26) complement the tools in clients that support them.

## Reversibility

Cheap to reverse: grouping is a naming and routing choice; splitting a group is a new tool and a CHANGELOG entry.

## Sources

- https://github.com/bytebase/dbhub (README, token measurements)
- https://www.reddit.com/r/mcp/comments/1kttr9n/ (HenkDz, 2025-05-23)
- https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool and https://code.claude.com/docs/en/agent-sdk/tool-search
- https://www.anthropic.com/engineering/writing-tools-for-agents (2025-09-11)
- https://claude.com/docs/connectors/building/review-criteria (retrieved 2026-09-12)
- `research/03-existing-postgresql-mcp-servers.md`, question 6; `research/06-mcp-tool-design-security-clients-distribution.md`, findings 2 to 8
