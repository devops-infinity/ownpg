# ADR-0002: Build OwnPG in Rust on the official rmcp SDK

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

The Principal Architect chose Rust and the current MCP specification for OwnPG. The MCP protocol layer can come from the official SDK, from a community crate, or from a hand-written implementation like the one in `reserve` (`crates/reserve-cli/src/server/mcp.rs`, about 1,600 lines for one tool over HTTP). OwnPG needs stdio and Streamable HTTP, dozens of tools with generated schemas, structured output, MRTR elicitation, cancellation, and conformance. Which crate carries the protocol?

## Decision Drivers

- Spec fidelity for `2026-07-28` and compatibility with `2025-11-25` clients.
- Maintenance signal: release cadence, conformance results, download count, and how production servers pin it.
- Tool authoring ergonomics: macros, schema generation, structured output, annotations.
- The smallest protocol surface OwnPG has to maintain itself.

## Options Considered

### Option A: rmcp 3.3.0, the official Rust SDK

Published 2026-09-10, Apache-2.0, MSRV 1.88, edition 2024, 25.9 million downloads, Tier 1 since 2026-08-21 with 100 percent conformance on both dated suites; implements `2026-07-28` and stays compatible with earlier revisions; `#[tool_router]`, `#[tool_handler]`, `Parameters<T>`, `Json<T>`, `ToolAnnotations`, runtime `disable_route`; stdio and Tower-based Streamable HTTP; `elicitation` and `request-state` features; 80 plus integration tests to copy patterns from; pinned at 3.3.0 by `apollo-mcp-server` and `pgmoneta_mcp`.

- Pros: official, Tier 1, same-day spec tracking, published versioning and dependency policies.
- Pros: the transport, header validation, MRTR plumbing, and legacy negotiation are already written and tested.
- Cons: server-side OAuth is not included; OwnPG writes the bearer middleware itself.
- Cons: two open stdio issues (#1261 first-request deadlock, #1030 unbounded line buffer) need workarounds and tests.
- Cons: `Option<T>` schema output trips some LLM hosts; schemas need care.

### Option B: rust-mcp-sdk 2.0.0 or another community crate

`rust-mcp-sdk` (2026-08-27, 267,899 downloads, 191 stars) reaches `2026-07-28` and ships server-side OAuth providers; `turbomcp` 4 is alpha; `tower-mcp` is tiny; the rest are stale or archived.

- Pros: server-side OAuth out of the box in `rust-mcp-sdk`.
- Cons: one maintainer group, 97 times fewer downloads, no tier commitment, and a second protocol implementation to trust.

### Option C: Hand-written protocol layer, extending the one in reserve

- Pros: total control; the dual-era logic already exists for HTTP.
- Cons: no stdio, no elicitation, no resources, no prompts, no structured output, no conformance history; every spec change lands on OwnPG's maintainer.

### Option D: Do nothing

- Cons: there is no server without a protocol layer.

## Decision

We will build OwnPG on `rmcp` 3.x, starting at 3.3.0 and resolving `latest` at install time.

It won on spec fidelity and maintenance signal, and because the parts it lacks (server-side bearer validation) are a few hundred lines of axum middleware, while the parts it provides would be months of work to rewrite and prove. The `reserve` MCP tests stay as the acceptance reference for dual-era behavior.

## Consequences

- Positive: OwnPG inherits conformance, header validation, MRTR, and legacy negotiation.
- Positive: tool authoring is macro-driven with generated schemas.
- Negative: OwnPG owns server-side OAuth (ADR-0014).
- Negative: stdio shipping waits on tests around issues #1261 and #1030.
- Neutral: feature flags to enable are `server`, `macros`, `transport-io`, `transport-streamable-http-server`, `elicitation`, `request-state`, `schemars`; the client-side `auth` features stay off.

## Reversibility

Expensive to reverse: the tool handlers are written against rmcp's traits and macros. Revisit if rmcp loses Tier 1, stops tracking the spec within a release cycle, or if `rust-mcp-sdk` becomes an official SDK.

## Sources

- https://crates.io/api/v1/crates/rmcp (read 2026-09-12)
- https://github.com/modelcontextprotocol/modelcontextprotocol/pull/3287 (merged 2026-08-21)
- https://raw.githubusercontent.com/modelcontextprotocol/rust-sdk/rmcp-v3.3.0/README.md
- https://github.com/modelcontextprotocol/rust-sdk/issues/1261 and /issues/1030
- `ownpg-research-report.md`, section 4; `research/02-rust-mcp-sdk-rmcp.md`
- `/Users/sharkar/Git-Repositories/reserve/crates/reserve-cli/src/server/mcp.rs:21-255` (read 2026-09-12)
