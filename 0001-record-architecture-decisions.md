# ADR-0001: Record architecture decisions

**Status**: Accepted
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

OwnPG starts from a research pack rather than from code, so its first decisions exist only in a report and a chat transcript. The `rust-cli-apps` blueprint (Group A) expects a written decision record for the crate split, the argument parser, the interface kind, the config format, the data store, and the distribution channels before implementation starts. Which format and location hold those records?

## Decision Drivers

- Future readers must find every decision cold, without the chat history.
- The house rules: plain US English, no tables, no em dashes, one continuous line per paragraph.
- The `writing-adrs` skill's template and lifecycle are already the house standard.
- Records must move unchanged from the planning folder into the repository.

## Options Considered

### Option A: Numbered ADR files on the house template

One file per decision, `NNNN-short-slug.md`, in the project root, with Status, Date, Authors, Decided by, Context, Decision Drivers, Options Considered (always including do nothing), Decision, Consequences, Reversibility, and Sources.

- Pros: matches the `writing-adrs` skill; each record is short and dated; superseding is explicit.
- Pros: the `rust-cli-apps` blueprint recognizes it.
- Cons: twenty files at the start is a lot of reading in one sitting.

### Option B: One decisions section inside the PRD

- Pros: one document.
- Cons: the PRD is per feature and gets edited; decisions would drift with it; no supersede trail.

### Option C: Do nothing

- Pros: no writing.
- Cons: the reasoning behind the SDK, driver, SSH, and safety choices would live only in the research report and the transcript, and be re-argued at every phase.

## Decision

We will record every significant decision as a numbered ADR file on the house template, kept in the project root next to the PRD and the plan.

Option A won because it is the standard the skills and the blueprint already expect, and because a superseded decision leaves a visible trail instead of an edited paragraph.

## Consequences

- Positive: each later phase starts from a dated record instead of a memory.
- Negative: an ADR is immutable once Accepted, so a changed mind costs a new file.
- Neutral: numbers are assigned once and never reused; 0002 through 0020 are assigned in this pack.

## Reversibility

Cheap to reverse: the files can be merged or moved within a sprint. Revisit only if the repository adopts a documentation system that carries its own decision log.

## Sources

- `~/.claude/skills/writing-adrs/SKILL.md`
- `~/.claude/skills/rust-cli-apps/references/blueprint-checklist.md`, Group A
- `ownpg-research-report.md`, section 9
