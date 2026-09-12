# ADR-0011: Destructive operations need a dry run, an explicit confirm, or a protocol confirmation

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

Agents run destructive SQL when asked loosely ("clean up the bad records"); pgEdge shipped custom write tools that ran without confirmation and received an advisory (2026-09-03); no surveyed server uses protocol elicitation for confirmation. The `2026-07-28` revision carries elicitation inside an `input_required` result with a sealed `requestState`, but the last client matrix lists elicitation only for Claude Code, Cursor, Codex, VS Code, and Copilot CLI, and not for Claude Desktop, claude.ai, Gemini CLI, Windsurf, or Zed. Anthropic's connector review requires `destructiveHint: true` on such tools and says they always prompt in Claude; the MCP blog reminds everyone that hints are hints. How does OwnPG gate destructive work?

## Decision Drivers

- The gate must work in every client, including ones without elicitation.
- The user must see the exact SQL before it runs.
- Confirmation state must survive a retry without becoming a replay vector.
- Annotations must be honest so clients that use them prompt correctly.

## Options Considered

### Option A: Three gates, layered

Every DDL and write tool accepts `dry_run` (returns the rendered SQL, runs nothing). Every statement the classifier marks destructive (ADR-0009) requires confirmation: if the client declared `elicitation`, the tool returns `resultType: "input_required"` with a boolean confirmation request and an HMAC-sealed `requestState` (rmcp `request-state` feature) bound to the statement hash, the principal, and a short TTL, and runs only after a yes; if the client did not declare elicitation, the tool refuses until the call carries `confirm: true` and names that argument and `dry_run` in the error. Destructive tools carry `destructiveHint: true`; additive writes carry `destructiveHint: false`; reads carry `readOnlyHint: true`; every tool carries `title` and `openWorldHint: false`.

- Pros: works everywhere; the protocol path is an upgrade, never a dependency.
- Pros: replay is blocked by the sealed state and TTL.
- Cons: two confirmation paths to test.

### Option B: Elicitation only

- Pros: the cleanest protocol story.
- Cons: Claude Desktop and claude.ai users could never confirm anything.

### Option C: A startup flag (`--allow-destructive`, as HenkDz does)

- Pros: simple.
- Cons: one flag opens every destructive path for the whole session; prompt fatigue turns it on permanently.

### Option D: Do nothing (rely on client prompts from annotations)

- Cons: annotations are untrusted hints; several clients do not prompt at all.

## Decision

We will gate every destructive statement behind three layered options: `dry_run` always available, MRTR elicitation when the client supports it, and an explicit `confirm: true` argument otherwise, with honest annotations on every tool.

Option A won because it is the only design that works in every client the Principal Architect uses today.

## Consequences

- Positive: a user sees the SQL, then says yes, in every client.
- Positive: the audit record carries the decision path (`confirmed_elicitation`, `confirmed_argument`, `dry_run`, `refused`).
- Negative: `requestState` sealing needs a key, generated per process and held in memory; only a multi-instance remote deployment reads one from a 0600 file in the data directory.
- Neutral: form-mode elicitation never asks for a secret, as the spec requires.

## Reversibility

Cheap to reverse per client as elicitation support spreads; the argument path can become a fallback only.

## Sources

- https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr and the elicitation page
- https://claude.com/docs/connectors/building/review-criteria (retrieved 2026-09-12)
- https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/ (2026-03-16)
- https://github.com/modelcontextprotocol/modelcontextprotocol/blob/87993a68/docs/clients.mdx (2026-05-26 snapshot)
- `research/03-existing-postgresql-mcp-servers.md`, question 3 (pgEdge advisory GHSA-v44x-hxg7-5cgc)
