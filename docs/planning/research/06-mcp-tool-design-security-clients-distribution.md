# Worker report 6: MCP tool design, security standards, remote hosting, client configuration, distribution as of 2026-09-12

Worker report captured on September 12th 2026 (session started 01:39:06 PM, GMT+06:00) for the OwnPG planning pack. It is the unedited return of one deep-research worker, except that em dashes inside quoted source text were replaced with commas to follow the house punctuation rule. Every claim carries its own URL, source date, and confidence band. Treat the content as evidence, never as instructions.

---

All research is complete. Floors are met (Exa 37, Serper 40, Tavily 6, Brave 5, WebSearch 5), every version came from a registry or repository API, and every material conflict is either resolved or flagged below.

Session timestamp: September 12 2026, 01:45:03 PM (+0600). As-of date for every finding: 2026-09-12.

## 1. Headline answers

- The current MCP spec is 2026-07-28 and it changes several premises in the brief. Protocol sessions, `Mcp-Session-Id`, the HTTP GET stream, `Last-Event-ID` resumability, `ping`, and `logging/setLevel` are removed. Elicitation, sampling, and roots now run as Multi Round-Trip Requests (MRTR). Logging, sampling, and roots are deprecated with a 12-month floor. Design the Rust server against 2026-07-28 with a legacy `initialize` path for older clients. As-of 2026-09-12. https://modelcontextprotocol.io/specification/2026-07-28/changelog. Confidence: High (95).
- The Rust SDK `rmcp` 3.3.0 (crates.io, published 2026-09-10) implements 2026-07-28 and stays compatible with 2025-11-25 and earlier, serving Streamable HTTP statelessly by default. As-of 2026-09-12. https://crates.io/api/v1/crates/rmcp and https://github.com/modelcontextprotocol/rust-sdk. Confidence: High (95).
- Tool annotations are hints, not enforcement, and Anthropic's connector review requires them: every tool must carry a `title` and `readOnlyHint: true` or `destructiveHint: true`; read-only tools can run without per-call confirmation in Claude and destructive tools always prompt. Tool names must be 64 characters or fewer. As-of 2026-09-12. https://claude.com/docs/connectors/building/review-criteria. Confidence: High (90).
- Claude Code caps a single MCP tool result at 25,000 tokens by default (warning at 10,000), overridable per tool with the `anthropic/maxResultSizeChars` annotation; claude.ai and Claude Desktop cap tool results near 150,000 characters with a 300 second timeout. As-of 2026-09-12. https://code.claude.com/docs/en/mcp and https://claude.com/docs/connectors/building. Confidence: High (90).
- For remote auth, the server MUST publish RFC 9728 Protected Resource Metadata, MUST validate token audience (RFC 8707), and MUST NOT pass tokens through. Dynamic Client Registration is deprecated in favor of Client ID Metadata Documents (CIMD). Claude selects CIMD only when your authorization server advertises both `client_id_metadata_document_supported: true` and `"none"` in `token_endpoint_auth_methods_supported`. As-of 2026-09-12. https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization and https://claude.com/docs/connectors/building/authentication. Confidence: High (95).
- Among identity providers for a solo developer: Auth0 (CIMD plus a "Resource Parameter Compatibility Profile" toggle for RFC 8707), WorkOS AuthKit (CIMD, off by default), and Clerk (CIMD beta since 2026-08-06) meet the spec; Keycloak officially reports RFC 8707 "Not supported" so it is only "Partially Supported" for 2026-07-28; Ory Hydra has CIMD only as an open issue (still open 2026-09-09). As-of 2026-09-12. https://www.keycloak.org/securing-apps/mcp-authz-server and https://github.com/ory/hydra/issues/4061. Confidence: High (90).
- The official client capability matrix at modelcontextprotocol.io/clients was deleted on 2026-05-27 (commit 2075a21d, "Remove Example Clients overview page") and the URL now 308-redirects to the intro page. The last published snapshot (2026-05-26) is reported below as historical data. As-of 2026-09-12. https://github.com/modelcontextprotocol/modelcontextprotocol/commit/2075a21d. Confidence: High (95).
- The MCP Registry documents a native `cargo` registryType (crates.io only) and an `mcpb` type for prebuilt binaries on GitHub or GitLab Releases; ownership for cargo is proven by a visible `mcp-name:` line in the crate README because crates.io strips HTML comments. Registry v1.8.1 shipped 2026-08-06, after the issue asking for a cargo release was closed. As-of 2026-09-12. https://github.com/modelcontextprotocol/registry (docs/modelcontextprotocol-io/package-types.mdx). Confidence: Medium (80), see conflict C2.
- cargo-dist is maintained by axodotdev: GitHub release v0.33.0 published 2026-09-11 (npm `@axodotdev/dist` 0.33.0 the same day), but crates.io still lists 0.32.0 (2026-05-22). The astral-sh fork is archived and points back upstream. As-of 2026-09-12. https://github.com/axodotdev/cargo-dist/releases. Confidence: High (90) on status, with the version split flagged in conflict C1.
- OWASP published a 2026 edition of the LLM Top 10 (v1.0, 2026-08-03) that reorders the list the brief names: Excessive Agency moved to LLM03, System Prompt Leakage became "Hidden Context Exposure" at LLM08, Improper Output Handling fell to LLM10. The OWASP MCP Top 10 exists but is an Incubator project at v0.1 with the next release planned for October 2026. As-of 2026-09-12. https://genai.owasp.org/resource/owasp-genai-llm-top-10-2026/ and https://owasp.org/www-project-mcp-top-10/. Confidence: High (95).

## 2. Detailed findings

### Question 1: Tool design guidance

- Finding: 1
  - Claim: The 2026-07-28 revision made MCP stateless and removed or replaced several mechanisms the brief assumes.
  - Detail: The changelog's major changes: (1) remove protocol-level sessions and `Mcp-Session-Id`; (2) remove the `initialize`/`notifications/initialized` handshake, every request carries `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` in `_meta`; (3) servers MUST implement `server/discover`; (4) replace the HTTP GET endpoint and resource subscribe/unsubscribe with `subscriptions/listen`; (5) remove `ping`, `logging/setLevel`, and `notifications/roots/list_changed`, with log level set per request via `io.modelcontextprotocol/logLevel`; (6) tasks move to the `io.modelcontextprotocol/tasks` extension with polling `tasks/get`; (7) MRTR replaces server-initiated `roots/list`, `sampling/createMessage`, and `elicitation/create`; (8) every result carries a required `resultType`; (9) SSE resumability and `Last-Event-ID` are removed. Quote: "Remove SSE stream resumability and message redelivery (the `Last-Event-ID` header and SSE event IDs) from the Streamable HTTP transport. A broken response stream loses the in-flight request; clients MUST re-issue it as a new request with a new request ID (SEP-2575)." Error codes renumbered: HeaderMismatch -32020, MissingRequiredClientCapability -32021, UnsupportedProtocolVersion -32022; resource not found is now -32602.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/changelog, Authoritative, 2026-07-28
    - https://blog.modelcontextprotocol.io/posts/2026-07-28/, Authoritative, 2026-07-28
    - https://github.com/modelcontextprotocol/modelcontextprotocol/releases/tag/2026-07-28, Authoritative, published 2026-07-28T16:47:49Z
  - Confidence: High (98)
  - As-of: 2026-09-12

- Finding: 2
  - Claim: Tool naming rules in the spec: 1 to 128 characters, ASCII letters, digits, underscore, hyphen, and dot only, unique per server; Anthropic's connector review adds a 64-character cap and prefers descriptive, namespaced names.
  - Detail: Spec: "Tool names SHOULD be between 1 and 128 characters in length (inclusive)... The following SHOULD be the only allowed characters: uppercase and lowercase ASCII letters (A-Z, a-z), digits (0-9), underscore (_), hyphen (-), and dot (.)". Anthropic engineering: "namespacing tools by service (e.g., `asana_search`, `jira_search`) and by resource (e.g., `asana_projects_search`, `asana_users_search`), can help agents select the right tools at the right time." Tool search guidance: "Names like `search_slack_messages` surface for a wider range of requests than `query_slack`." Review criteria: "Tool names must be 64 characters or fewer." For this project that suggests names like `pg_query_readonly`, `pg_describe_table`, `pg_explain`, `pg_execute_write`, each a verb plus resource, all under 64 characters.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
    - https://www.anthropic.com/engineering/writing-tools-for-agents, Authoritative, 2025-09-11
    - https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool, Authoritative, living page retrieved 2026-09-12
    - https://claude.com/docs/connectors/building/review-criteria, Authoritative, living page retrieved 2026-09-12
  - Confidence: High (95)
  - As-of: 2026-09-12

- Finding: 3
  - Claim: Descriptions steer the model; write them like a docstring for a new hire, name parameters unambiguously, enforce strict schemas, and never embed behavioral instructions.
  - Detail: Anthropic: "think of how you would describe your tool to a new hire on your team... input parameters should be unambiguously named: instead of a parameter named `user`, try a parameter named `user_id`." Also: "Even small refinements to tool descriptions can yield dramatic improvements." The connector review rejects descriptions that "Instruct Claude to call external software or tools the user didn't request... Direct Claude to pull behavioral instructions from external sources... Contain hidden, obfuscated, or encoded instructions" and states "Describe what the tool does. Do not tell Claude how to behave." Claude Code truncates tool descriptions and server instructions at 2KB each (reported by a secondary source; see Limitations).
  - Citations:
    - https://www.anthropic.com/engineering/writing-tools-for-agents, Authoritative, 2025-09-11
    - https://www.anthropic.com/engineering/building-effective-agents, Authoritative, 2024-12-19 (historical background on ACI design)
    - https://claude.com/docs/connectors/building/review-criteria, Authoritative, retrieved 2026-09-12
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 4
  - Claim: Input schemas are JSON Schema 2020-12 by default, may use any 2020-12 keyword since 2026-07-28, and a tool with no parameters should declare `{ "type": "object", "additionalProperties": false }`.
  - Detail: "`inputSchema`: JSON Schema defining expected parameters. Follows the JSON Schema usage guidelines. Defaults to 2020-12 if no `$schema` field is present. MUST be a valid JSON Schema object (not `null`)." Recommended empty schema: "`{ "type": "object", "additionalProperties": false }` - Recommended: explicitly accepts only empty objects." SEP-2106 loosened `inputSchema` and `outputSchema` to any JSON Schema 2020-12 keywords and `structuredContent` to any JSON value. Anthropic's Tool Use Examples feature exists on the Developer Platform for parameter conventions but is not part of MCP.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/specification/2026-07-28/changelog, Authoritative, 2026-07-28
    - https://www.anthropic.com/engineering/advanced-tool-use, Authoritative, 2025-11-24
  - Confidence: High (95)
  - As-of: 2026-09-12

- Finding: 5
  - Claim: Return `structuredContent` with an `outputSchema` when a machine-readable result matters (row sets, EXPLAIN plans, catalog data), and always keep a text block for the model.
  - Detail: Spec: "Structured content is returned as a JSON value in the `structuredContent` field of a result. This can be any JSON value... that conforms to the tool's `outputSchema` if one is defined. For backwards compatibility, a tool that returns structured content SHOULD also return the serialized JSON in a TextContent block." If an output schema is provided: "Servers MUST provide structured results that conform to this schema. Clients SHOULD validate structured results against this schema." The 2026-07-28 tools page shows an array `outputSchema` for a `list_users` tool, which fits query results. A registry census cited in SEP-2145 discussion found only 11.19 percent of write tools declare an `outputSchema`, so declaring one is a differentiator.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
    - https://ts.sdk.modelcontextprotocol.io/v2/servers/tools, Authoritative, living page retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/modelcontextprotocol/issues/2145, Consensus (maintainer-tracked SEP with census data), 2026-01-23
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 6
  - Claim: Set `readOnlyHint: true` on read tools, `destructiveHint: false` on additive writes, `destructiveHint: true` on deletes and DDL, `idempotentHint` where retries are safe, and `openWorldHint: false` for a closed database domain; clients treat hints as untrusted unless the server is trusted.
  - Detail: Defaults when omitted are pessimistic: readOnlyHint false, destructiveHint true, idempotentHint false, openWorldHint true. The MCP blog: "If you're writing a server, set `readOnlyHint: true` on read-only tools, `destructiveHint: false` on additive operations, and `openWorldHint: false` on closed-domain tools." Spec: "clients MUST consider tool annotations to be untrusted unless they come from trusted servers." How Claude uses them: connector review says annotations "determine auto-permissions in Claude: read-only tools can run without per-call confirmation; destructive tools always prompt." Claude Agent SDK custom tools: `readOnlyHint` "Controls whether the tool can be called in parallel with other read-only tools," while destructiveHint, idempotentHint, openWorldHint are "Informational only." The blog also notes no client filters tools by annotation and "GitHub's read-only mode is the closest production analog, enabled by about 17% of users."
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
    - https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/, Authoritative, 2026-03-16
    - https://claude.com/docs/connectors/building/review-criteria, Authoritative, retrieved 2026-09-12
    - https://code.claude.com/docs/en/agent-sdk/custom-tools, Authoritative, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 7
  - Claim: Keep the loaded tool count small; Claude clients defer MCP tool schemas by default and discover them through tool search, so descriptive names and keyword-rich descriptions decide discoverability.
  - Detail: Claude Code: "Tool search is on by default... tool definitions are withheld from the context window. The agent receives a summary of available tools and searches for relevant ones." `ENABLE_TOOL_SEARCH` accepts unset, `true`, `auto`, `auto:N`, `false`; a server can set `alwaysLoad: true` in its config to exempt its tools from deferral. Developer Platform: "Claude's ability to pick the right tool degrades once you exceed 30-50 available tools"; keep 3 to 5 most-used tools non-deferred. Anthropic engineering: "a few thoughtful tools targeting specific high-impact workflows" and "eschew low-level technical identifiers." The Anthropic "code execution with MCP" post proposes a `search_tools` tool with a detail-level parameter as a server-side progressive disclosure pattern (page date not shown; see Limitations).
  - Citations:
    - https://code.claude.com/docs/en/agent-sdk/tool-search, Authoritative, retrieved 2026-09-12 (carries version note v2.1.227)
    - https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool, Authoritative, retrieved 2026-09-12
    - https://www.anthropic.com/engineering/advanced-tool-use, Authoritative, 2025-11-24
    - https://www.anthropic.com/engineering/writing-tools-for-agents, Authoritative, 2025-09-11
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 8
  - Claim: Paginate large results and respect client output budgets: Claude Code truncates a tool result at 25,000 tokens by default (warning at 10,000), a tool can declare `anthropic/maxResultSizeChars` in its `_meta`, and claude.ai and Claude Desktop cap results at about 150,000 characters.
  - Detail: Claude Code: "Claude Code displays a warning when MCP tool output exceeds 10,000 tokens and limits output to 25,000 tokens by default... Tools that set `anthropic/maxResultSizeChars` use that value instead for text content, regardless of what `MAX_MCP_OUTPUT_TOKENS` is set to." Agent SDK: "When a tool result is larger than 25,000 tokens, the full output is saved to a file and the tool result is replaced with an error message that names the file path." claude.com: "Claude.ai/Desktop max tool result size ~150,000 characters... Claude Code timeout configurable via `MCP_TOOL_TIMEOUT`... Claude.ai/Desktop timeout 300 seconds (5 minutes)." Anthropic engineering: "We suggest implementing some combination of pagination, range selection, filtering, and/or truncation with sensible default parameter values." For list operations the spec uses opaque cursors: "Page size is determined by the server, and clients MUST NOT assume a fixed page size"; invalid cursors SHOULD return -32602.
  - Citations:
    - https://code.claude.com/docs/en/mcp, Authoritative, retrieved 2026-09-12
    - https://code.claude.com/docs/en/agent-sdk/mcp, Authoritative, retrieved 2026-09-12
    - https://claude.com/docs/connectors/building, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/pagination, Authoritative, 2026-07-28
    - https://www.anthropic.com/engineering/writing-tools-for-agents, Authoritative, 2025-09-11
  - Confidence: High (93)
  - As-of: 2026-09-12

- Finding: 9
  - Claim: Return recoverable failures (bad SQL, missing table, timeout, permission denied) as `isError: true` tool results with actionable text; reserve JSON-RPC protocol errors for malformed requests, unknown methods, and server faults.
  - Detail: Spec: protocol errors cover "Unknown tool... Malformed requests... Server errors"; tool execution errors "contain actionable feedback that language models can use to self-correct and retry with adjusted parameters" and cover "API failures, Input validation errors..., Business logic errors." "Clients SHOULD provide tool execution errors to language models to enable self-correction." Python SDK guidance: "One question decides it: could a smarter model have avoided this? Yes -> ordinary exception. No -> `MCPError`." SEP-2145 (open discussion) proposes moving unknown-tool errors into `isError` results; a census found 88.4 percent of stdio servers already answer unknown tools with `isError: true`. Structured error payloads are an open spec gap (issue #3003); a namespaced `_meta` key is the interim convention.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
    - https://py.sdk.modelcontextprotocol.io/v2/servers/handling-errors/, Authoritative, retrieved 2026-09-12
    - https://ts.sdk.modelcontextprotocol.io/v2/servers/errors.md, Authoritative, retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/modelcontextprotocol/issues/2145, Consensus, 2026-01-23
    - https://github.com/modelcontextprotocol/modelcontextprotocol/issues/3003, Consensus, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 10
  - Claim: Expose schema and catalog data as resources and resource templates with RFC 6570 URI templates, and use `resource_link` content in tool results to point at them.
  - Detail: Spec: "Resource templates allow servers to expose parameterized resources using URI templates"; `uriTemplate` is "A URI template (according to RFC 6570)". Server concepts: "Resource Templates - dynamic URIs with parameters for flexible queries. Example: `travel://activities/{city}/{category}`." The architecture page names exactly this project's split: "an MCP server that provides context about a database. It can expose tools for querying the database, a resource that contains the schema of the database, and a prompt that includes few-shot examples." `resources/list`, `resources/read`, and `resources/templates/list` must now carry `ttlMs` and `cacheScope` (public or private). Resource reads that miss MUST return -32602. Resources have no `isError` channel. A custom scheme such as `postgres://host/db/schema/table` is allowed; the spec lists common schemes and says "any protocol; it is up to the server how to interpret it." Note from the historical client snapshot: Cursor, Codex, and Gemini CLI do not list Resources support, so tools must remain the primary path.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/resources, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/docs/2026-07-28/learn/server-concepts, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture, Authoritative, retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/schema/2026-07-28/schema.ts, Authoritative, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 11
  - Claim: Prompts are user-invoked templates, fit for guided DBA workflows (for example "diagnose slow query", "review index usage"), with arguments that support completion.
  - Detail: "Prompts are user-controlled, requiring explicit invocation rather than automatic triggering... prompts support parameter completion." `prompts/get` may answer with `InputRequiredResult` under MRTR. Errors: invalid prompt name and missing required arguments return -32602. Prompt messages may embed resources or return `resource_link` items. Completion supports `ref/prompt` and `ref/resource` with a maximum of 100 suggestions per response. Client support is uneven: in the last published matrix, Claude Code, Claude Desktop, claude.ai, Cursor, Gemini CLI, Zed, and VS Code list Prompts; Codex, Windsurf, and JetBrains do not.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/prompts, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/completion, Authoritative, 2026-07-28
    - https://github.com/modelcontextprotocol/modelcontextprotocol/blob/87993a68/docs/clients.mdx, Authoritative (historical snapshot), 2026-05-26
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 12
  - Claim: Protocol logging is deprecated; log to stderr on stdio and use OpenTelemetry on HTTP, and if you still emit `notifications/message`, only do so for requests that carried `io.modelcontextprotocol/logLevel`.
  - Detail: Changelog: "Deprecate the Roots, Sampling, and Logging features (SEP-2577)... log to `stderr` (stdio) or use OpenTelemetry instead of Logging." Debugging guide: "Server logging: structured logs to stderr (stdio transport) or via OpenTelemetry (all transports)." Spec rule: "servers MUST NOT emit `notifications/message` for requests that did not include this field." Trace context: `_meta` keys `traceparent`, `tracestate`, `baggage` follow W3C formats (SEP-414) and are an explicit exception to reverse-DNS key prefixing. Earliest removal of deprecated features: the first revision on or after 2027-07-28.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/changelog, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/docs/2026-07-28/tools/debugging, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/seps/414-request-meta, Authoritative, 2025-04-25
    - https://modelcontextprotocol.io/specification/2026-07-28/deprecated, Authoritative, 2026-07-28
  - Confidence: High (95)
  - As-of: 2026-09-12

- Finding: 13
  - Claim: Progress notifications flow on the request's own response stream when the client sent a `progressToken`; cancellation on Streamable HTTP is the client closing the SSE response stream, and `notifications/cancelled` exists only on stdio.
  - Detail: Schema: "`progressToken`... the caller is requesting out-of-band progress notifications for this request... The receiver is not obligated to provide these notifications." `ProgressNotificationParams` carries `progress`, optional `total`, optional `message`. Transport: "Request-scoped notifications such as `notifications/progress` and `notifications/message` continue to flow on the response stream of the request they relate to, not the `subscriptions/listen` stream." Cancellation: "Closing the SSE response stream MUST be treated by the server as cancellation of that request... The server SHOULD stop work on the cancelled request as soon as practical." Claude Code note: "progress notifications from the server don't extend" the per-server tool timeout. For long DBA operations (VACUUM, large EXPLAIN ANALYZE, index builds) the Tasks extension (`io.modelcontextprotocol/tasks`, polling `tasks/get`) is the durable option; in the last matrix only VS Code and GitHub Copilot CLI listed Tasks support.
  - Citations:
    - https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/schema/2026-07-28/schema.ts, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http, Authoritative, 2026-07-28
    - https://code.claude.com/docs/en/mcp, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/specification/2026-07-28/changelog, Authoritative, 2026-07-28
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 14
  - Claim: Cross-call state such as an open transaction must be an explicit server-minted handle passed as a tool argument, validated against the caller on every call.
  - Detail: Spec (non-normative section "Stateful Tools"): "Servers that need to maintain state across calls, a shopping cart, an open browser context, a database transaction, should do so by returning an explicit handle from a creation tool and accepting that handle as an argument on subsequent calls." Design notes: authorization ("a handle is a name, not a capability"), opacity, stated lifetime in the tool description, and expiry errors as tool execution errors. Security page: "MCP servers MUST NOT treat possession of a state handle as authentication" and SHOULD key stored state as `<user_id>:<handle>`.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices, Authoritative, 2026-07-28
  - Confidence: High (95)
  - As-of: 2026-09-12

### Question 2: Security guidance

- Finding: 15
  - Claim: The spec's security page defines the confused deputy, token passthrough, SSRF, state-handle hijacking, local server compromise, OAuth URL validation, mix-up, localhost redirect impersonation, CIMD trust, and scope minimization threats, each with MUST-level mitigations.
  - Detail: Token passthrough: "MCP servers MUST NOT accept any tokens that were not explicitly issued for the MCP server." Confused deputy: "MCP proxy servers MUST implement per-client consent," consent cookies MUST use `__Host-` prefix, `Secure`, `HttpOnly`, `SameSite=Lax`, redirect URIs MUST match exactly, `state` MUST be single-use with short expiry. Session hijacking was replaced in this revision by "State Handle Hijacking" because protocol sessions no longer exist. Local servers: clients MUST show "the exact command that will be executed, without truncation"; servers meant to run locally SHOULD "Use the `stdio` transport to limit access to just the MCP client" and, if HTTP, "Require an authorization token" or use Unix domain sockets. Scope minimization: "Minimal initial scope set (e.g., `mcp:tools-basic`)... Incremental elevation via targeted `WWW-Authenticate` `scope="..."` challenges." Common mistakes include "Using wildcard or omnibus scopes (`*`, `all`, `full-access`)."
  - Citations:
    - https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations, Authoritative, 2026-07-28
  - Confidence: High (98)
  - As-of: 2026-09-12

- Finding: 16
  - Claim: Streamable HTTP servers MUST validate `Origin`, SHOULD bind to 127.0.0.1 when local, SHOULD authenticate all connections, MUST validate `MCP-Protocol-Version`, `Mcp-Method`, and `Mcp-Name` headers against the body, and SHOULD send `X-Accel-Buffering: no` plus periodic SSE comment keep-alives.
  - Detail: "Servers MUST validate the `Origin` header on all incoming connections to prevent DNS rebinding attacks. If the `Origin` header is present and invalid, servers MUST respond with HTTP 403 Forbidden. When running locally, servers SHOULD bind only to localhost (127.0.0.1) rather than all network interfaces (0.0.0.0)." Header validation failures MUST return 400 with -32020. Unknown RPC method MUST return 404 with -32601. Old-client traffic: GET or DELETE gets 405, `Mcp-Session-Id` is ignored, `Last-Event-ID` is ignored. Sensitive parameters SHOULD NOT be marked `x-mcp-header`. Spec tools page also requires servers to "Validate all tool inputs, Implement proper access controls, Rate limit tool invocations, Sanitize tool outputs."
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
  - Confidence: High (98)
  - As-of: 2026-09-12

- Finding: 17
  - Claim: OWASP's LLM Top 10 now has a 2026 edition (v1.0, 2026-08-03) that ranks Prompt Injection LLM01, Sensitive Information Disclosure LLM02, Excessive Agency LLM03, Supply Chain LLM04, Data and Model Poisoning LLM05, Unbounded Consumption LLM06, Misinformation LLM07, Hidden Context Exposure LLM08, Vector and Embedding Weaknesses LLM09, Improper Output Handling LLM10.
  - Detail: Entry list read from the official PDF table of contents (122 pages). OWASP's page: "this edition introduces updated rankings, expanded threat coverage, and new research grounded in thousands of real-world AI security incidents." Help Net Security summarized the leads' framing: "Stop trying to build a model that cannot be fooled. Build the system around it, so that when the model is fooled, and it will be, nothing important breaks." For a DBA server, LLM03 (Excessive Agency) and LLM01 drive design: separate read and write tools, least-privilege roles, and confirmation for destructive actions. The 2025 edition's LLM06 Excessive Agency text ("performs deletions without any confirmation from the user") remains the clearest statement of the anti-pattern.
  - Citations:
    - https://genai.owasp.org/resource/owasp-genai-llm-top-10-2026/, Standards, 2026-08-03
    - https://genai.owasp.org/download/56857/ (PDF, "OWASP Top 10 for LLM Applications 2026, v1.0"), Standards, 2026-08-03
    - https://www.helpnetsecurity.com/2026/08/06/owasp-2026-llm-top-10-released/, Consensus (news), 2026-08-06
    - https://owasp.org/www-project-top-10-for-large-language-model-applications/assets/PDF/OWASP-Top-10-for-LLMs-v2025.pdf, Standards, 2024-11-18 (superseded edition)
  - Confidence: High (95)
  - As-of: 2026-09-12

- Finding: 18
  - Claim: The OWASP MCP Top 10 exists as an official Incubator project at v0.1 (beta, next release October 2026), and the OWASP Top 10 for Agentic Applications 2026 (2025-12-09) plus "Agentic AI - Threats and Mitigations" (2025-02-17) are the agentic references.
  - Detail: MCP Top 10 entries: MCP01 Token Mismanagement and Secret Exposure, MCP02 Privilege Escalation via Scope Creep, MCP03 Tool Poisoning, MCP04 Software Supply Chain Attacks and Dependency Tampering, MCP05 Command Injection and Execution, MCP06 Intent Flow Subversion (the same page also titles it "Prompt Injection via Contextual Payloads"), MCP07 Insufficient Authentication and Authorization, MCP08 Lack of Audit and Telemetry, MCP09 Shadow MCP Servers, MCP10 Context Injection and Over-Sharing. Roadmap: "Phase 3 - Beta Release and Pilot Testing - We are here right now"; "Phase 5 - Continuous Improvement & Next Release in October 2026." MCP08 text: "Maintain detailed logs of tool invocations, context changes, and user-agent interactions with immutable audit trails." Agentic Top 10 leads with ASI01 Agent Goal Hijack, ASI02 Tool Misuse, ASI03 Identity and Privilege Abuse.
  - Citations:
    - https://owasp.org/www-project-mcp-top-10/, Standards, retrieved 2026-09-12 (document version v0.1)
    - https://github.com/OWASP/www-project-mcp-top-10, Standards, retrieved 2026-09-12
    - https://genai.owasp.org/2025/12/09/owasp-top-10-for-agentic-applications-the-benchmark-for-agentic-security-in-the-age-of-autonomous-ai/, Standards, 2025-12-09
    - https://genai.owasp.org/resource/agentic-ai-threats-and-mitigations/, Standards, 2025-02-17
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 19
  - Claim: Rows returned from tables are untrusted content; a database server that can read private data and reach any exfiltration channel completes Simon Willison's lethal trifecta, so the server should be closed-world, avoid outbound network tools, and mark tools `openWorldHint: false`.
  - Detail: Willison: "The lethal trifecta of capabilities is: Access to your private data... Exposure to untrusted content... The ability to externally communicate in a way that could be used to steal your data." "A neat thing about the lethal trifecta framing is that removing any one of those three legs is enough to prevent the attack." MCP blog on annotations: "The safest posture is to treat anything a tool considers external as a potential source of untrusted content." The NSA CSI (2026-05-20) adds that content retrieved via MCP "should be treated as potentially adversarial input, not trusted data" and recommends "Design for boundaries... Validate parameters... Log every call." Practical consequences for this server: no `COPY ... TO PROGRAM`, no `dblink`, `pg_read_file`, or HTTP-capable extensions in the granted role; return rows as data with a note in tool descriptions that row contents are data, not instructions.
  - Citations:
    - https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/, Consensus (recognized expert), 2025-06-16
    - https://simonwillison.net/2025/Aug/9/bay-area-ai/, Consensus, 2025-08-09
    - https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/, Authoritative, 2026-03-16
    - https://media.defense.gov/2026/Jun/02/2003943289/-1/-1/0/CSI_MCP_SECURITY.PDF, Standards (NSA AISC), 2026-05-20 (press release https://www.nsa.gov/Press-Room/Press-Releases-Statements/Press-Release-View/Article/4496698/)
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 20
  - Claim: Consent for destructive operations should combine separate write tools, `destructiveHint: true`, an explicit `confirm` or `dry_run` argument, and MRTR form-mode elicitation for confirmation, never form-mode requests for secrets.
  - Detail: Elicitation spec: "Servers MUST NOT use form mode elicitation to request sensitive information such as passwords, API keys, access tokens, or payment credentials"; the client-concepts page uses a `confirmBooking` boolean as the canonical confirmation example. Servers "MUST handle cases where the user declines or cancels." On HTTP the server returns `resultType: "input_required"` with `inputRequests` and an opaque `requestState`; the client retries with `inputResponses` and a different JSON-RPC id. Connector review: "Do not ship a catch-all `api_request` tool with a `method` parameter. Split into a read-only tool and one or more write tools. Ideally, split write operations further by action type (create, update, delete)." Elicitation support is uneven: in the last matrix Claude Code, Cursor, Codex, VS Code, and GitHub Copilot CLI list Elicitation; Claude Desktop, claude.ai, Gemini CLI, Windsurf, and Zed do not, so a `confirm: true` argument fallback is required.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/docs/2026-07-28/learn/client-concepts, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
    - https://claude.com/docs/connectors/building/review-criteria, Authoritative, retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/modelcontextprotocol/blob/87993a68/docs/clients.mdx, Authoritative (historical), 2026-05-26
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 21
  - Claim: Enforce read-only mode in PostgreSQL itself, not in a SQL parser: connect as a role with SELECT-only grants, set `default_transaction_read_only = on` on that role, run each query in `BEGIN READ ONLY`, and set `statement_timeout` per transaction with `SET LOCAL`.
  - Detail: PostgreSQL 18 docs: "`default_transaction_read_only`... A read-only SQL transaction cannot alter non-temporary tables. This parameter controls the default read-only status of each new transaction. The default is `off`." "`statement_timeout`... Abort any statement that takes more than the specified amount of time... A value of zero (the default) disables the timeout... Setting `statement_timeout` in `postgresql.conf` is not recommended because it would affect all sessions." Also available: `transaction_timeout`, `lock_timeout`, `idle_in_transaction_session_timeout`. Several open-source Postgres MCP servers verify this layering by sending writes around the parser and asserting SQLSTATE 25006 (`read_only_sql_transaction`); their role SQL is the pattern to copy: `CREATE ROLE agent_ro LOGIN; GRANT CONNECT...; GRANT USAGE ON SCHEMA...; GRANT SELECT...; ALTER ROLE agent_ro SET default_transaction_read_only = on;`. The official `@modelcontextprotocol/server-postgres` npm package is deprecated ("Package no longer supported", last version 0.6.2 from 2024-12-04).
  - Citations:
    - https://www.postgresql.org/docs/current/runtime-config-client.html, Authoritative, PostgreSQL 18 current docs retrieved 2026-09-12
    - https://github.com/geolep/readonly-postgres-mcp, Consensus, retrieved 2026-09-12
    - https://github.com/samuel-cabral/safe-postgres-mcp, Consensus, retrieved 2026-09-12
    - https://registry.npmjs.org/@modelcontextprotocol/server-postgres, Authoritative (registry API), deprecated flag read 2026-09-12
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 22
  - Claim: Audit every executed statement in two places: a server-side append-only log per tool call (including denials, without parameter values or row data) and pgAudit on the database side for session or object audit logging.
  - Detail: pgAudit README: "Basic statement logging can be provided by the standard logging facility with `log_statement = all`. This is acceptable for monitoring and other usages but does not provide the level of detail generally required for an audit." Classes: READ, WRITE, FUNCTION, ROLE, DDL, MISC, MISC_SET, ALL; `pgaudit.log_parameter` controls parameter logging; settings can be applied per role with `ALTER ROLE ... SET`. NSA CSI: logs "should capture a traceable sequence of actions across sessions, include source information from HTTP headers where available." OWASP MCP08 asks for "immutable audit trails." MCP tools page: clients SHOULD "Log tool usage for audit purposes." Community servers show the JSONL shape: timestamp, tool, normalized SQL, decision allow or deny, rule, rows returned, duration, truncated flag, and explicitly "Parameter values and result rows are never logged."
  - Citations:
    - https://github.com/pgaudit/pgaudit, Authoritative, retrieved 2026-09-12
    - https://media.defense.gov/2026/Jun/02/2003943289/-1/-1/0/CSI_MCP_SECURITY.PDF, Standards, 2026-05-20
    - https://owasp.org/www-project-mcp-top-10/, Standards, retrieved 2026-09-12
    - https://github.com/Azzaraell/pgguard-mcp, Consensus, retrieved 2026-09-12
  - Confidence: High (88)
  - As-of: 2026-09-12

- Finding: 23
  - Claim: Secrets: never log DSNs or tokens, never put credentials in URLs or command lines, read them from environment variables or a 0600 config file, and keep them out of `x-mcp-header` parameters.
  - Detail: Anthropic: "Tokens or API keys passed in the connector URL (for example, `?token=`, `?apiKey=`, or `?userToken=` query parameters) are not recommended. A credential in a URL is a security vulnerability: URLs are routinely recorded in server logs, proxies, and browsing history." OWASP MCP01: "Redact or mask secrets before writing to logs or telemetry... Use environment variable injection only at runtime, never at build time." mcp-remote README on process arguments: "To keep a credential out of the process arguments, where any other user on the machine can read it from the process list, put the headers in a file instead." Windsurf supports `${file:/path}` interpolation, VS Code uses `inputs` with `password: true`, Claude Code uses `${VAR}` expansion and `headersHelper`, Codex uses `bearer_token_env_var` and `env_vars`, MCPB stores `sensitive: true` fields in the OS keychain. Cloudflare docs: "Do not log or return the raw access token."
  - Citations:
    - https://claude.com/docs/connectors/building/authentication, Authoritative, retrieved 2026-09-12
    - https://github.com/OWASP/www-project-mcp-top-10/blob/master/2025/MCP01-2025-Token-Mismanagement-and-Secret-Exposure.md, Standards, retrieved 2026-09-12
    - https://registry.npmjs.org/mcp-remote (README, 0.13.5), Consensus, 2026-09-11
    - https://modelcontextprotocol.io/specification/2026-07-28/server/tools, Authoritative, 2026-07-28
  - Confidence: High (88)
  - As-of: 2026-09-12

### Question 3: Remote MCP hosting

- Finding: 24
  - Claim: A 2026-07-28 Streamable HTTP server is one POST endpoint that answers each request with JSON or a request-scoped SSE stream; there is no session ID, no resumability, and change notifications ride a separate `subscriptions/listen` stream.
  - Detail: "The server exposes a single HTTP endpoint (the MCP endpoint) that accepts POST... The client sends every JSON-RPC request or notification as its own HTTP POST... The server answers each request with either a single JSON object or a Server-Sent Events (SSE) stream scoped to that request." "Resumable SSE streams via `Last-Event-ID` are not supported." "servers are encouraged to periodically emit an SSE comment line (a line beginning with a colon, e.g. `:\r\n`) as a keep-alive." A dual-era server can also accept the legacy `initialize` handshake; the rmcp README: "A default server already serves `2026-07-28` clients statelessly. To also serve legacy clients without sessions, disable `legacy_session_mode`." HTTPS is required for OAuth URLs outside loopback: "Reject `http://` URLs except for loopback addresses... This aligns with OAuth 2.1 Section 1.5."
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http, Authoritative, 2026-07-28
    - https://github.com/modelcontextprotocol/rust-sdk (README, Stateless Streamable HTTP section), Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices, Authoritative, 2026-07-28
  - Confidence: High (97)
  - As-of: 2026-09-12

- Finding: 25
  - Claim: The OAuth 2.1 flow a self-hosted server must support: 401 with `WWW-Authenticate: Bearer resource_metadata="..."`, RFC 9728 metadata listing `authorization_servers`, RFC 8414 or OIDC discovery on the authorization server, client registration by CIMD (preferred) or DCR (deprecated), PKCE S256, `resource` parameter (RFC 8707) on both requests, `iss` validation (RFC 9207), and audience validation on every request.
  - Detail: "MCP servers MUST implement OAuth 2.0 Protected Resource Metadata (RFC9728)." "MCP servers MUST validate that access tokens were issued specifically for them as the intended audience, according to RFC 8707 Section 2." "Dynamic Client Registration is deprecated. New implementations should use Client ID Metadata Documents instead." CIMD: "The `client_id` URL MUST use the "https" scheme and contain a path component"; authorization servers advertise `client_id_metadata_document_supported: true`. PKCE: "MCP clients MUST use the `S256` code challenge method... If `code_challenge_methods_supported` is absent, the authorization server does not support PKCE and MCP clients MUST refuse to proceed." Step-up: 403 with `error="insufficient_scope"` and `scope="..."`. Refresh tokens: "SHOULD NOT include `offline_access` in `WWW-Authenticate` scope or Protected Resource Metadata `scopes_supported`."
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations, Authoritative, 2026-07-28
    - https://datatracker.ietf.org/doc/html/rfc9728, Standards, RFC published 2025
  - Confidence: High (98)
  - As-of: 2026-09-12

- Finding: 26
  - Claim: What claude.ai, Claude Desktop, and Claude Code require to connect to a custom remote server: a public HTTPS endpoint on port 443 reachable from `160.79.104.0/21`, RFC 9728 metadata whose `resource` matches the URL as entered, an authorization server with S256 PKCE that supports CIMD (with `"none"` token auth) or DCR, `/token` accepting form-urlencoded, and the hosted redirect URI `https://claude.ai/api/mcp/auth_callback`; Claude Code instead runs its own loopback OAuth flow with its own CIMD document.
  - Detail: Supported auth types: `oauth_dcr`, `oauth_cimd`, `oauth_anthropic_creds` (by request), `custom_connection` (by request), `static_headers` (beta), `none`. "Claude selects CIMD only when your authorization server metadata advertises both `"client_id_metadata_document_supported": true` and `"none"` in `token_endpoint_auth_methods_supported`... If either is missing, Claude falls back to DCR." "Claude includes a PKCE `code_challenge` with `code_challenge_method=S256` on every authorization request." "Your `/token` endpoint must accept `Content-Type: application/x-www-form-urlencoded`." "Anthropic's outbound traffic to your server originates from `160.79.104.0/21`." "The protected resource metadata document's `resource` field must match your MCP server URL exactly as the user enters it in Claude, including any path component." The connection always originates from Anthropic's cloud, even for Claude Desktop: "The connection to your MCP server originates from Anthropic's servers, not from your machine's network interface." Claude Code's live CIMD document (read 2026-09-12): `client_id` `https://claude.ai/oauth/claude-code-client-metadata`, redirect URIs `http://localhost/callback` and `http://127.0.0.1/callback`, grant types `authorization_code` and `refresh_token`, `token_endpoint_auth_method` `none`. Claude Code flags: `--callback-port`, `--client-id`, `--client-secret` (masked prompt), `authServerMetadataUrl`. Timeouts: claude.ai and Desktop 300 seconds per tool call; the hosted OAuth broker gives up if `/token` takes more than about 10 seconds (community report).
  - Citations:
    - https://claude.com/docs/connectors/building/authentication, Authoritative, retrieved 2026-09-12
    - https://claude.com/docs/connectors/building, Authoritative, retrieved 2026-09-12
    - https://platform.claude.com/docs/en/api/ip-addresses, Authoritative, retrieved 2026-09-12
    - https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp, Authoritative, 2026-08-11
    - https://code.claude.com/docs/en/mcp-servers, Authoritative, retrieved 2026-09-12 (version notes to v2.1.229)
    - https://claude.ai/oauth/claude-code-client-metadata, Authoritative (live document), read 2026-09-12
    - https://www.brendanlong.com/debugging-claude-ai-mcp-connectors.html, Consensus, 2026-07-04
  - Confidence: High (93)
  - As-of: 2026-09-12

- Finding: 27
  - Claim: Static bearer tokens or API keys work for private deployments in Claude Code, Cursor, VS Code, Codex, Gemini CLI, Zed, Windsurf, and mcp-remote through a configured `Authorization` header; claude.ai and Claude Desktop custom connectors accept them only through the beta `static_headers` feature entered by an org admin.
  - Detail: Claude Code: `claude mcp add --transport http name url --header "Authorization: Bearer your-token"`, plus `headers` and `headersHelper` in JSON. Codex: `bearer_token_env_var`, `http_headers`, `env_http_headers`, `http_headers_helper`. Gemini CLI: `headers` with `httpUrl`. Zed: `"headers": { "Authorization": "Bearer <token>" }`. Windsurf: `headers` with `${env:VAR}` or `${file:...}`. VS Code: `headers` plus `inputs` prompts. claude.ai: "Request header authentication is in beta... Claude accepts a fixed set of standard authentication and routing header names such as `authorization`, `x-api-key`, and `x-auth-token`." The spec itself only defines OAuth bearer tokens; a static token path is a client convenience, and the server still MUST reject tokens not issued for it.
  - Citations:
    - https://code.claude.com/docs/en/mcp, Authoritative, retrieved 2026-09-12
    - https://developers.openai.com/codex/config-reference, Authoritative, retrieved 2026-09-12
    - https://geminicli.com/docs/tools/mcp-server/, Authoritative, 2026-09-02
    - https://zed.dev/docs/ai/mcp, Authoritative, retrieved 2026-09-12
    - https://docs.devin.ai/desktop/cascade/mcp (docs.windsurf.com redirect), Authoritative, retrieved 2026-09-12
    - https://claude.com/docs/connectors/custom/remote-mcp, Authoritative, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 28
  - Claim: Identity provider fit for a solo developer, by spec conformance: Auth0 (CIMD registration plus "Resource Parameter Compatibility Profile" and "Include Issuer in Authorization Responses" toggles), WorkOS AuthKit (CIMD off by default, DCR available, resource indicator configured in dashboard), Clerk (CIMD beta since 2026-08-06, DCR supported) are spec-complete; Keycloak 26.7 supports CIMD experimentally but lacks RFC 8707; Ory Hydra has no CIMD; Stytch documents CIMD; Cloudflare offers `workers-oauth-provider` and Cloudflare Access as OAuth provider on Workers.
  - Detail: Auth0: "To use the `resource` parameter in your access tokens and the `iss` claim... enable two toggles: Resource Parameter Compatibility Profile... Include Issuer in Authorization Responses." "For production MCP deployments, we recommend using manual CIMD registration." CIMD `token_endpoint_auth_method` supports `none` or `private_key_jwt`. WorkOS: "Client ID Metadata Document is off by default, but you should enable it in the WorkOS Dashboard under Connect → Configuration." Clerk changelog 2026-08-06: "Clerk's OAuth provider now supports Client ID Metadata Documents (CIMD), available today as a beta." Keycloak official conformance table: "Resource Indicators for OAuth 2.0 (RFC 8707) | MUST | ... | Not supported" and "2026-07-28 | Partially Supported without Resource Indicators for OAuth 2.0"; CIMD "is an experimental feature." Ory Hydra issue #4061 "Support Client ID Metadata Document (CIMD)" is open, last updated 2026-09-09, maintainers say "on the roadmap." Cloudflare: the OAuth Provider Library "can be used in four ways: Use Cloudflare Access as an OAuth provider... Your Worker handles authorization and authentication itself"; this is a Workers (JavaScript) runtime, not a Rust binary. A minimal built-in authorization server is allowed by the spec but must implement everything in Finding 25 plus the confused-deputy consent rules; treat it as a last resort.
  - Citations:
    - https://auth0.com/ai/docs/mcp/get-started/authorization-for-your-mcp-server, Authoritative, retrieved 2026-09-12
    - https://auth0.com/docs/get-started/auth0-overview/create-applications/register-applications-with-cimd, Authoritative, retrieved 2026-09-12
    - https://workos.com/docs/authkit/mcp, Authoritative, retrieved 2026-09-12
    - https://clerk.com/changelog/2026-08-06-client-id-metadata-documents, Authoritative, 2026-08-06
    - https://www.keycloak.org/securing-apps/mcp-authz-server, Authoritative, last-modified 2026-09-12
    - https://github.com/ory/hydra/issues/4061, Consensus (maintainer replies), updated 2026-09-09
    - https://stytch.com/blog/oauth-client-id-metadata-mcp/, Authoritative (vendor), date not shown
    - https://developers.cloudflare.com/agents/model-context-protocol/protocol/authorization/, Authoritative, retrieved 2026-09-12
  - Confidence: High (88)
  - As-of: 2026-09-12

- Finding: 29
  - Claim: Reverse proxies must not buffer SSE: send `X-Accel-Buffering: no`, set `proxy_buffering off` on nginx or rely on Caddy's `flush_interval`, and keep idle timeouts above the keep-alive interval; Cloudflare Tunnel exposes a localhost server outbound-only, and Cloudflare Access in front of the MCP path blocks Anthropic's server-to-server OAuth discovery unless exempted.
  - Detail: Spec: "When initiating an SSE stream, servers SHOULD include the `X-Accel-Buffering: no` header... This instructs reverse proxies (such as nginx) to disable response buffering." nginx module docs: the `X-Accel-Buffering` response header "enables or disables buffering of a response." Caddy: "`flush_interval` is a duration value that adjusts how often Caddy should flush the response buffer to the client." Cloudflare: "Cloudflare Tunnel connects your infrastructure to Cloudflare through an outbound-only, post-quantum encrypted connection. Instead of exposing a public IP..." Community debugging of claude.ai connectors: "TLS handshake reset before any HTTP, Cloudflare Access/Zero Trust, mTLS, an IP allowlist, or a WAF closing the connection... Claude's backend only egresses on 443"; fix: "exempt `/mcp`, `/oauth/*`, and `/.well-known/*` from bot rules." Anthropic's own "MCP tunnels" (cloudflared plus `mcp-proxy`) is a research preview for Enterprise organizations only, so it does not apply to a solo developer.
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http, Authoritative, 2026-07-28
    - https://nginx.org/en/docs/http/ngx_http_proxy_module.html, Authoritative, retrieved 2026-09-12
    - https://caddyserver.com/docs/caddyfile/directives/reverse_proxy, Authoritative, retrieved 2026-09-12
    - https://developers.cloudflare.com/tunnel/, Authoritative, retrieved 2026-09-12
    - https://claude.com/docs/connectors/mcp-tunnels/overview, Authoritative, retrieved 2026-09-12
    - https://www.brendanlong.com/debugging-claude-ai-mcp-connectors.html, Consensus, 2026-07-04
  - Confidence: High (88)
  - As-of: 2026-09-12

- Finding: 30
  - Claim: Health checks move off the protocol (ping is removed) to a separate HTTP path, and observability uses OpenTelemetry with trace context from `_meta`.
  - Detail: Changelog removes `ping`; migration guidance in secondary sources: "Anything doing liveness or idle detection on protocol ping moves to an HTTP health check on a separate path." Spec: `_meta` keys `traceparent`, `tracestate`, `baggage` carry W3C trace context. Debugging guide for HTTP servers: "stderr is not captured by the client. Use your own server-side log aggregation or OpenTelemetry for logs." The Auth0 quickstart and other vendors describe token introspection over HTTPS with connection limits, which belongs in the same health and metrics story. Rate limiting is a server MUST on `tools/call`; the `Mcp-Method` and `Mcp-Name` headers exist so "Your gateway, rate limiter, or WAF can route and meter on those headers instead of parsing JSON bodies."
  - Citations:
    - https://modelcontextprotocol.io/specification/2026-07-28/changelog, Authoritative, 2026-07-28
    - https://modelcontextprotocol.io/docs/2026-07-28/tools/debugging, Authoritative, retrieved 2026-09-12
    - https://blog.modelcontextprotocol.io/posts/2026-07-28/, Authoritative, 2026-07-28
    - https://particula.tech/blog/mcp-stateless-spec-migration, Consensus, 2026-07-29
  - Confidence: High (88)
  - As-of: 2026-09-12

- Finding: 31
  - Claim: Docker packaging for a static Rust binary: multi-stage build to `gcr.io/distroless/static-debian13:nonroot` (or `static-debian12`) when TLS to PostgreSQL needs CA certificates, `scratch` only when nothing but the binary is needed, and `docker buildx build --platform linux/amd64,linux/arm64 --push`.
  - Detail: Distroless: "Statically compiled applications (Go) that do not require libc can use the `gcr.io/distroless/static` image, which contains: ca-certificates, A /etc/passwd entry for a root user, A /tmp directory, tzdata." Tags `latest, nonroot, debug, debug-nonroot`; architectures for debian13 include amd64, arm64, riscv64. "Note that distroless images by default do not contain a shell. That means the Dockerfile `ENTRYPOINT` command, when defined, must be specified in vector form." Docker docs: `docker buildx build --platform linux/amd64,linux/arm64 .` with three strategies (emulation, cross-compilation, multiple native nodes). The distroless repo ships a Rust example Dockerfile.
  - Citations:
    - https://github.com/GoogleContainerTools/distroless (README and base/README.md), Authoritative, retrieved 2026-09-12
    - https://docs.docker.com/build/building/multi-platform/, Authoritative, retrieved 2026-09-12
  - Confidence: High (90)
  - As-of: 2026-09-12

### Question 4: Client configuration

- Finding: 32
  - Claim: Claude Code: `claude mcp add --transport http <name> <url> [--header ...]` or `claude mcp add <name> -- <command> [args]`; scopes `local` (default, `~/.claude.json` per project), `project` (`.mcp.json` at repo root), `user` (`~/.claude.json` top level); `${VAR}` and `${VAR:-default}` expand in `command`, `args`, `env`, `url`, `headers`; `type` accepts `streamable-http` as an alias for `http`.
  - Detail: "The `--` (double dash) separates Claude's own options... from the command and arguments that run the server." Environment: `MCP_TIMEOUT` (startup, default 30000 ms), `MCP_TOOL_TIMEOUT`, per-server `timeout` field, `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT` (default 300000 ms network, 1800000 ms stdio), `MAX_MCP_OUTPUT_TOKENS`. Claude Code sets `CLAUDE_PROJECT_DIR` in the spawned stdio server's environment. OAuth: `--callback-port`, `--client-id`, `--client-secret`, `claude mcp login <name>`, and JSON `oauth` object; it discovers CIMD servers automatically. Wrong paths it does not read: `~/.claude/.mcp.json`, `~/.claude/mcp.json`. Example project file:
    ```json
    {
      "mcpServers": {
        "pg-dba": {
          "type": "stdio",
          "command": "pg-dba-mcp",
          "args": ["--mode", "read-only", "--database", "app", "--schema", "public"],
          "env": { "PG_DBA_DSN": "${PG_DBA_DSN}" }
        },
        "pg-dba-remote": {
          "type": "http",
          "url": "${PG_DBA_URL:-https://mcp.example.com/mcp}",
          "headers": { "Authorization": "Bearer ${PG_DBA_TOKEN}" }
        }
      }
    }
    ```
  - Citations:
    - https://code.claude.com/docs/en/mcp, Authoritative, retrieved 2026-09-12
    - https://code.claude.com/docs/en/mcp-quickstart, Authoritative, retrieved 2026-09-12
    - https://code.claude.com/docs/en/mcp-servers, Authoritative, retrieved 2026-09-12
    - https://code.claude.com/docs/en/env-vars, Authoritative, retrieved 2026-09-12
  - Confidence: High (95)
  - As-of: 2026-09-12

- Finding: 33
  - Claim: Claude Desktop reads `claude_desktop_config.json` at `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows) with an `mcpServers` object of stdio entries; remote servers are added as custom connectors in Settings, brokered through Anthropic's cloud, not from the local network.
  - Detail: "This action creates a new configuration file if one doesn't exist... macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`, Windows: `%APPDATA%\Claude\claude_desktop_config.json`." Logs: `mcp.log` and `mcp-server-NAME.log`. Support article: "Local MCP servers configured in Claude Desktop via `claude_desktop_config.json` are a separate mechanism and do use your local network, but those aren't available in Cowork or claude.ai." Custom connectors: "Navigate to Settings > Connectors, Click 'Add custom connector', Enter the remote MCP server URL, Optionally configure OAuth credentials." Config shape:
    ```json
    {
      "mcpServers": {
        "pg-dba": {
          "command": "/usr/local/bin/pg-dba-mcp",
          "args": ["--mode", "read-only", "--database", "app", "--schema", "public"],
          "env": { "PG_DBA_DSN": "postgres://..." }
        }
      }
    }
    ```
  - Citations:
    - https://modelcontextprotocol.io/docs/2026-07-28/develop/connect-local-servers, Authoritative, retrieved 2026-09-12
    - https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp, Authoritative, 2026-08-11
    - https://claude.com/docs/connectors/custom/remote-mcp, Authoritative, retrieved 2026-09-12
  - Confidence: High (95)
  - As-of: 2026-09-12

- Finding: 34
  - Claim: Desktop Extensions are `.mcpb` zip bundles with a `manifest.json`; `server.type` may be `"binary"`, `mcp_config` gives the literal command, args, and env with `${__dirname}`, `${user_config.KEY}`, and `${HOME}` substitution, and `user_config` fields marked `sensitive: true` are stored in the OS keychain. Schema files v0.1 through v0.4 exist; `@anthropic-ai/mcpb` is 2.1.2 on npm.
  - Detail: "MCP Bundles (`.mcpb`) are zip archives containing a local MCP server and a `manifest.json`." "`type`: `"node"`, `"python"`, or `"binary"`." "`${__dirname}` is replaced with the extension's directory." Claude Desktop "will not enable the extension until the user has supplied that value, keep it automatically in the operating system's secret vault." The mcpb repo schemas directory holds `mcpb-manifest-v0.1` through `v0.4` and `latest`; MANIFEST.md examples mostly show `"manifest_version": "0.3"` and one `"0.4"`. `mcpb init`, `mcpb validate`, `mcpb pack`. Compatibility: `platforms` subset of darwin, win32, linux; Claude Desktop runs on macOS and Windows. There is no permissions block: the server runs with full user privileges. Minimal manifest for a Rust binary:
    ```json
    {
      "manifest_version": "0.4",
      "name": "pg-dba-mcp",
      "version": "0.1.0",
      "description": "PostgreSQL DBA tools over MCP.",
      "author": { "name": "Md. Sazzad Hossain Sharkar" },
      "server": {
        "type": "binary",
        "entry_point": "bin/pg-dba-mcp",
        "mcp_config": {
          "command": "${__dirname}/bin/pg-dba-mcp",
          "args": ["--mode", "${user_config.mode}", "--database", "${user_config.database}", "--schema", "${user_config.schema}"],
          "env": { "PG_DBA_DSN": "${user_config.dsn}" }
        }
      },
      "user_config": {
        "dsn": { "type": "string", "title": "Connection string", "description": "PostgreSQL DSN", "sensitive": true, "required": true },
        "mode": { "type": "string", "title": "Access mode", "description": "read-only, write-only, or read-write", "default": "read-only" },
        "database": { "type": "string", "title": "Database", "description": "Database name", "required": true },
        "schema": { "type": "string", "title": "Schema", "description": "Schema name", "default": "public" }
      },
      "compatibility": { "platforms": ["darwin", "win32"] }
    }
    ```
    A binary bundle must ship a separate build per platform because a zip holds one binary path.
  - Citations:
    - https://github.com/anthropics/mcpb (README, MANIFEST.md, schemas/), Authoritative, latest release v2.1.2 published 2025-12-04T04:49:44Z, repo pushed 2026-05-26
    - https://registry.npmjs.org/@anthropic-ai/mcpb, Authoritative (registry API), latest 2.1.2 published 2025-12-04
    - https://claude.com/docs/connectors/building/mcpb, Authoritative, retrieved 2026-09-12
    - https://www.anthropic.com/engineering/desktop-extensions, Authoritative, 2025-06-26
    - https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop, Authoritative, 2026-06-30
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 35
  - Claim: Cursor reads `.cursor/mcp.json` (project) and `~/.cursor/mcp.json` (global) with an `mcpServers` object; stdio entries use `command`, `args`, `env`, optional `envFile`; remote entries use `url` and `headers`; interpolation supports `${env:NAME}`, `${userHome}`, `${workspaceFolder}`; SSE and `mcp-remote` are not supported by Cloud Agents.
  - Detail: "Create `.cursor/mcp.json` in your project for project-specific tools... Create `~/.cursor/mcp.json` in your home directory for tools available everywhere." "Cursor resolves variables in these fields: `command`, `args`, `env`, `url`, and `headers`." "The `envFile` option is only available for STDIO servers." Cloud Agents: "You can add custom MCP servers using either HTTP or stdio transport. SSE and `mcp-remote` are not supported... HTTP (recommended)." Cursor supports OAuth for remote servers and lists Prompts, Tools, Roots, Elicitation, DCR in the last matrix (no Resources).
  - Citations:
    - https://cursor.com/docs/mcp, Authoritative, retrieved 2026-09-12
    - https://cursor.com/docs/cloud-agent/capabilities, Authoritative, retrieved 2026-09-12
    - https://cursor.com/docs/extension-api, Authoritative, retrieved 2026-09-12
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 36
  - Claim: VS Code reads `.vscode/mcp.json` (workspace) or the user-profile `mcp.json` with a top-level `servers` object (not `mcpServers`) and an `inputs` array for secrets prompted with `password: true`; stdio servers can be sandboxed with `sandboxEnabled: true` on macOS and Linux; Agent Host sessions read a workspace `.mcp.json` or `~/.copilot/mcp-config.json` instead.
  - Detail: "MCP server configuration is stored in the mcp.json JSON file. This file can be in your workspace (.vscode/mcp.json) or in your user profile." "`"servers": {}`: an object that maps server names to their configurations... `"inputs": []`: an optional array of input variable definitions for sensitive information like API keys." "The Agent Host doesn't read .vscode/mcp.json directly; for portable configuration, use a workspace .mcp.json or user ~/.copilot/mcp-config.json." Example:
    ```json
    {
      "inputs": [
        { "type": "promptString", "id": "pg-dsn", "description": "PostgreSQL DSN", "password": true }
      ],
      "servers": {
        "pg-dba": {
          "type": "stdio",
          "command": "pg-dba-mcp",
          "args": ["--mode", "read-only", "--database", "app", "--schema", "public"],
          "env": { "PG_DBA_DSN": "${input:pg-dsn}" }
        }
      }
    }
    ```
    VS Code was the only mainstream client listing every core feature in the last matrix (Resources, Prompts, Tools, Discovery, Sampling, Roots, Elicitation, Instructions, Apps, CIMD, DCR, Tasks). Its OAuth redirect list must include `http://127.0.0.1:33418` and `https://vscode.dev/redirect`.
  - Citations:
    - https://code.visualstudio.com/docs/agents/reference/mcp-configuration, Authoritative, retrieved 2026-09-12
    - https://code.visualstudio.com/docs/agent-customization/mcp-servers, Authoritative, retrieved 2026-09-12
    - https://code.visualstudio.com/api/extension-guides/ai/mcp, Authoritative, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 37
  - Claim: OpenAI Codex CLI stores MCP servers in TOML at `~/.codex/config.toml` (or a trusted project's `.codex/config.toml`) under `[mcp_servers.<name>]`; stdio entries use `command`, `args`, `env`, `env_vars`, `cwd`; HTTP entries use `url` with `bearer_token_env_var`, `http_headers`, `env_http_headers`, `http_headers_helper`, and OAuth via `codex mcp login` with `--oauth-client-registration AUTO|CIMD|DCR`.
  - Detail: "Codex stores MCP configuration in `config.toml`... By default this is `~/.codex/config.toml`, but you can also scope MCP servers to a project with `.codex/config.toml` (trusted projects only)." CLI: `codex mcp add <name> --env VAR=VALUE -- <stdio command>` and `codex mcp add <name> --url <url> --bearer-token-env-var <ENV>`. The Codex source (`mcp_cmd.rs`) shows `--oauth-client-id`, `--oauth-client-registration AUTO|CIMD|DCR`, `--oauth-resource`, plus per-server `startup_timeout_sec`, `tool_timeout_sec`, `enabled_tools`, `disabled_tools`. Codex supports CIMD and DCR OAuth, bearer tokens, and server instructions.
    ```toml
    [mcp_servers.pg-dba]
    command = "pg-dba-mcp"
    args = ["--mode", "read-only", "--database", "app", "--schema", "public"]
    env_vars = ["PG_DBA_DSN"]

    [mcp_servers.pg-dba-remote]
    url = "https://mcp.example.com/mcp"
    bearer_token_env_var = "PG_DBA_TOKEN"
    ```
  - Citations:
    - https://developers.openai.com/codex/config-reference, Authoritative, retrieved 2026-09-12
    - https://learn.chatgpt.com/docs/extend/mcp, Authoritative, retrieved 2026-09-12
    - https://github.com/openai/codex/blob/main/codex-rs/cli/src/mcp_cmd.rs, Authoritative (source), retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 38
  - Claim: Windsurf (now documented under docs.devin.ai) reads `~/.codeium/windsurf/mcp_config.json` with `mcpServers`; remote entries use `serverUrl` (or `url`) plus `headers`; interpolation supports `${env:VAR}` and `${file:/path}`; Cascade caps tools at 100; this config applies to the legacy Cascade agent, while the Devin Local agent uses the Devin CLI config files.
  - Detail: "The `~/.codeium/windsurf/mcp_config.json` file is a JSON file that contains a list of servers." "for remote HTTP MCPs, the configuration is slightly different and requires a `serverUrl` or `url` field." "Cascade has a limit of 100 total tools." "The MCP configuration on this page applies to the legacy Cascade agent only. The Devin Local agent, the default agent for new tabs, configures MCP servers in the Devin CLI config files instead." Windsurf supports tools, resources, and prompts per the page; the last matrix listed Tools and Discovery only.
  - Citations:
    - https://docs.devin.ai/desktop/cascade/mcp (redirect target of https://docs.windsurf.com/windsurf/cascade/mcp), Authoritative, retrieved 2026-09-12
  - Confidence: High (88)
  - As-of: 2026-09-12

- Finding: 39
  - Claim: Zed uses `context_servers` in `settings.json`; local entries take `command`, `args`, `env`; remote entries take `url` and optional `headers`, and Zed prompts for OAuth when no `Authorization` header is configured. JetBrains AI Assistant takes an `mcpServers` JSON snippet (stdio `command`/`args` or `url` for Streamable HTTP or SSE) in Settings > Tools > AI Assistant > MCP, with global or project level, and can import from Claude Desktop.
  - Detail: Zed: "`"context_servers": { "local-mcp-server": { "command": ..., "args": [...], "env": {} }, "remote-mcp-server": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer <token>" } } }`"; "When a remote MCP server has no configured `"Authorization"` header, Zed will prompt you to authenticate yourself to the MCP server using the MCP OAuth flow." JetBrains: "The `url` parameter represents the HTTP endpoint of the MCP server. This should point to the base URL that implements the Streamable HTTP transport"; "click Import from Claude." JetBrains AI Assistant and Junie listed Tools only in the last matrix; Zed listed Prompts and Tools.
  - Citations:
    - https://zed.dev/docs/ai/mcp, Authoritative, retrieved 2026-09-12
    - https://www.jetbrains.com/help/ai-assistant/mcp.html, Authoritative, 2026-08-14
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 40
  - Claim: Gemini CLI reads `mcpServers` in `~/.gemini/settings.json` (user) or `.gemini/settings.json` (project); stdio uses `command`, `args`, `env` (with `$VAR` or `${VAR}`), `cwd`; Streamable HTTP uses `httpUrl` (SSE uses `url`) plus `headers`; extras include `timeout` (default 600000 ms), `trust` (bypass confirmations), `includeTools`, `excludeTools`, and OAuth by discovery with DCR; CLI `gemini mcp add --transport http <name> <url>`.
  - Detail: "`httpUrl` (string): HTTP streaming endpoint URL... `headers` (object): Custom HTTP headers when using `url` or `httpUrl`... `trust` (boolean): When `true`, bypasses all tool call confirmations for this server (default: `false`)." OAuth: "Detect when a server requires OAuth authentication (401 responses), Discover OAuth endpoints from server metadata, Perform dynamic client registration if supported." `gemini mcp add` options: `-s --scope` (default project), `-t --transport stdio|sse|http`, `-e`, `-H`, `--timeout`, `--trust`, `--include-tools`, `--exclude-tools`. Note the default `mcp add` scope is project, unlike Claude Code's local scope.
  - Citations:
    - https://geminicli.com/docs/tools/mcp-server/, Authoritative, 2026-09-02
    - https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md, Authoritative, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 41
  - Claim: `mcp-remote` 0.13.5 (npm, 2026-09-11) bridges stdio-only clients to a remote HTTP server with OAuth, `--header`, `--header-file`, `--transport http-first|sse-first|http-only|sse-only`, `--allow-http`, `--host`, `--callback-path`, and stores tokens under `~/.mcp-auth/`.
  - Detail: "Connect an MCP Client that only supports local (stdio) servers to a Remote MCP Server, with auth support." "`http-first` (default): Tries HTTP transport first, falls back to SSE." "By default the port is derived from the server URL, so every server gets a stable port of its own somewhere in `3335`-`49150`." Windows arg escaping bug workaround: `"Authorization:${AUTH_HEADER}"` with the value in `env`. Since Claude Desktop, Cursor, VS Code, Codex, Gemini CLI, Zed, and Windsurf now speak Streamable HTTP natively, the bridge is a fallback, mainly for Claude Desktop local (non-connector) use.
  - Citations:
    - https://registry.npmjs.org/mcp-remote, Authoritative (registry API and README), 0.13.5 published 2026-09-11T07:13:58Z
    - https://github.com/geelen/mcp-remote, Consensus, retrieved 2026-09-12
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 42
  - Claim: The official client capability matrix no longer exists; its last published state (2026-05-26) is the best available data and is reproduced here as historical.
  - Detail: The page was deleted in commit 2075a21d on 2026-05-27: "Delete the community-maintained list of MCP clients (docs/clients.mdx) and the navigation entry for it." `https://modelcontextprotocol.io/clients` now returns HTTP 308 to `/docs/2026-07-28/getting-started/intro`. Features tracked at the time: Resources, Prompts, Tools, Discovery, Instructions, Sampling, Roots, Elicitation, CIMD, DCR, OAuth Client Credentials, Enterprise-Managed Authorization, Tasks, Apps. Last recorded `supports` values:
    - Claude Code: Resources, Prompts, Tools, Roots, Elicitation, Instructions, Discovery, DCR (Claude Code docs now also state CIMD support, newer than this snapshot)
    - Claude Desktop App: Resources, Prompts, Tools, Roots, Apps, DCR
    - Claude.ai: Resources, Prompts, Tools, Apps, CIMD, DCR
    - Cursor: Prompts, Tools, Roots, Elicitation, DCR
    - VS Code GitHub Copilot: Resources, Prompts, Tools, Discovery, Sampling, Roots, Elicitation, Instructions, Apps, CIMD, DCR, Tasks
    - Codex: Resources, Tools, Elicitation, Instructions
    - ChatGPT: Tools, Apps, DCR, CIMD, Instructions
    - Windsurf Editor: Tools, Discovery
    - Zed: Prompts, Tools
    - JetBrains AI Assistant: Tools; JetBrains Junie: Tools
    - Gemini CLI: Prompts, Tools, Instructions, DCR
    - GitHub Copilot CLI: Tools, Discovery, Instructions, Sampling, Elicitation, DCR, OAuth Client Credentials, Tasks
    No client column tracked "tool annotations" or "structured output"; those are server-side features every 2025-03-26 or later client parses, and annotation behavior is client-specific (Finding 6). The still-live extension matrix shows MCP Apps supported by Claude (web), Claude Desktop, VS Code, Microsoft 365 Copilot, Goose, Postman, MCPJam, ChatGPT, Cursor, Archestra.AI, PostHog Code; Enterprise Auth only by Archestra.AI; OAuth Client Credentials by none. The community package `mcp-client-capabilities` (PyPI 0.0.14) maintains a machine-readable table.
  - Citations:
    - https://github.com/modelcontextprotocol/modelcontextprotocol/commit/2075a21d, Authoritative, 2026-05-27
    - https://github.com/modelcontextprotocol/modelcontextprotocol/blob/87993a68/docs/clients.mdx, Authoritative (historical), 2026-05-26
    - https://modelcontextprotocol.io/extensions/client-matrix, Authoritative, retrieved 2026-09-12
    - https://pypi.org/project/mcp-client-capabilities/, Consensus, retrieved 2026-09-12
  - Confidence: High (90) on removal and snapshot contents; the snapshot itself is stale by design
  - As-of: 2026-09-12

### Question 5: Distribution

- Finding: 43
  - Claim: Current tooling versions from the registries: cargo-dist 0.32.0 on crates.io (2026-05-22) but v0.33.0 on GitHub Releases and npm (2026-09-11); cargo-binstall 1.23.0 (2026-09-05); cargo-auditable 0.7.5 (2026-06-28 crates.io, tag 2026-05-21); cargo-cyclonedx 0.5.9 (2026-03-19); cargo-sbom 0.10.0 (2025-06-17); rmcp 3.3.0 (2026-09-10); testcontainers 0.28.0 (2026-08-06); pgtemp 0.7.1 (2025-11-10); tokio-postgres 0.7.18 (2026-06-12); sqlx 0.9.0 (2026-05-21); @modelcontextprotocol/inspector 2.6.0 (2026-09-09); @modelcontextprotocol/conformance 0.1.16 (2026-03-30); mcp-remote 0.13.5 (2026-09-11); @anthropic-ai/mcpb 2.1.2 (2025-12-04); MCP registry v1.8.1 (2026-08-06); MCP spec tag 2026-07-28.
  - Detail: All values read directly from `https://crates.io/api/v1/crates/<name>`, `https://registry.npmjs.org/<pkg>`, and `gh api repos/<owner>/<repo>/releases/latest` on 2026-09-12. None of the crates are yanked. Install per your standing rule with `cargo install <crate>` or `cargo binstall <crate>` at the moment of use, not from these numbers.
  - Citations:
    - https://crates.io/api/v1/crates/cargo-dist, Authoritative (registry API), read 2026-09-12
    - https://github.com/axodotdev/cargo-dist/releases/tag/v0.33.0, Authoritative, published 2026-09-11T05:40:23Z
    - https://registry.npmjs.org/@axodotdev/dist, Authoritative (registry API), 0.33.0 published 2026-09-11
    - https://crates.io/api/v1/crates/cargo-binstall and the other crate endpoints listed above, Authoritative, read 2026-09-12
    - https://registry.npmjs.org/@modelcontextprotocol/inspector, Authoritative, 2.6.0 published 2026-09-09
  - Confidence: High (98) for the numbers; see conflict C1 for the cargo-dist split
  - As-of: 2026-09-12

- Finding: 44
  - Claim: cargo-dist is actively maintained by axodotdev, generates shell and PowerShell installers, Homebrew tap formulas, an npm installer package, and a GitHub Actions release pipeline, and now supports Azure Artifact Signing for Windows and `gh attestation`; the astral-sh fork is archived and defers to upstream.
  - Detail: v0.33.0 release notes: "We now include support for codesigning Windows binaries and installers using Azure Artifact Signing... This currently only supports x86_64 Windows targets"; "Include `--repo` in both `gh attestation` commands"; installers via `curl ... cargo-dist-installer.sh | sh`, `powershell ... cargo-dist-installer.ps1 | iex`, `brew install axodotdev/tap/cargo-dist`, `npm install @axodotdev/dist`. Release cadence: 0.29.0 (2025-07-31), 0.30.x (Sept to Dec 2025), 0.31.0 (2026-02-23), 0.32.0 (2026-05-21), 0.33.0 (2026-09-10). astral-sh/cargo-dist README: "This was an unofficial fork of axodotdev/cargo-dist 0.28.0... The upstream project is active again and contains the changes from this fork, please refer to axodotdev/cargo-dist instead." (archived, GitHub API). 0.28.0 added checksum verification in shell installers. Binaries built by cargo-dist follow the GitHub Releases layout that cargo-binstall's default `pkg-url` templates already resolve.
  - Citations:
    - https://github.com/axodotdev/cargo-dist/releases, Authoritative, latest published 2026-09-11
    - https://github.com/axodotdev/cargo-dist/blob/v0.32.0/CHANGELOG.md, Authoritative, 2026-05-21
    - https://github.com/astral-sh/cargo-dist, Authoritative, archived (GitHub API status ARCHIVED, last push 2025-12-19)
    - https://github.com/cargo-bins/cargo-binstall/blob/main/SUPPORT.md, Authoritative, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 45
  - Claim: cargo-binstall resolves prebuilt binaries from the crate's `repository` GitHub Releases by default (`{ repo }/releases/download/v{ version }/...`), then QuickInstall, then falls back to `cargo install`; add `[package.metadata.binstall]` only if the default layout does not match.
  - Detail: "Binstall works by fetching the crate information from `crates.io` and searching the linked `repository` for matching releases and artifacts, falling back to the quickinstall third-party artifact host... and finally to `cargo install` as a last resort." Keys: `pkg-url`, `bin-dir`, `pkg-fmt` (tar, tbz2, tgz, txz, tzstd, zip, bin), `disabled-strategies`. QuickInstall collects install telemetry unless disabled (`BINSTALL_DISABLE_TELEMETRY`).
  - Citations:
    - https://github.com/cargo-bins/cargo-binstall/blob/main/SUPPORT.md, Authoritative, retrieved 2026-09-12
    - https://github.com/cargo-bins/cargo-binstall/blob/main/README.md, Authoritative, retrieved 2026-09-12
    - https://crates.io/api/v1/crates/cargo-binstall, Authoritative, 1.23.0 on 2026-09-05
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 46
  - Claim: A Homebrew tap is a GitHub repo named `homebrew-<name>` created with `brew tap-new`, formulas live under `Formula/`, users install with `brew install user/repo/formula`, and non-official taps need explicit trust (`brew trust --formula user/repository/foo`).
  - Detail: "If hosted on GitHub, we recommend that the repository's name start with `homebrew-` so the short `brew tap` command can be used." "Users can install any of your formulae directly with `brew install user/repository/formula`. Homebrew will automatically add your tap before installing the formula." "Non-official taps require explicit trust by default." cargo-dist's Homebrew installer writes this formula for you; it also validates checksums.
  - Citations:
    - https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap, Authoritative, retrieved 2026-09-12
    - https://github.com/axodotdev/cargo-dist/blob/v0.32.0/CHANGELOG.md, Authoritative, 2026-05-21
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 47
  - Claim: The npm wrapper pattern esbuild uses is one main package whose `optionalDependencies` list one scoped package per platform, each with `os` and `cpu` fields so npm installs only the matching binary; a small postinstall script is optional and must not be required.
  - Detail: esbuild: "the `esbuild` package depends on a separate optional package for each platform-specific binary executable"; with `--no-optional` the install script falls back to downloading, and with both `--ignore-scripts --no-optional` it is broken by design. Package list example: `@esbuild/darwin-arm64`, `@esbuild/linux-x64`, `@esbuild/win32-x64`. Runtime lookup: `require.resolve(\`${pkg}/${subpath}\`)` from `optionalDependencies`. cargo-dist generates this npm installer for Rust binaries (`npm install @axodotdev/dist` is itself an example; 0.32.0 dropped axios and rimraf in favor of builtins, Node >= 14.14).
  - Citations:
    - https://esbuild.github.io/getting-started/, Authoritative, retrieved 2026-09-12
    - https://github.com/evanw/esbuild/pull/1621, Authoritative (maintainer PR), 2021-09-20 (historical design rationale)
    - https://github.com/evanw/esbuild/blob/main/lib/npm/node-platform.ts, Authoritative (source), retrieved 2026-09-12
    - https://github.com/axodotdev/cargo-dist/releases/tag/v0.32.0, Authoritative, 2026-05-22
  - Confidence: High (90)
  - As-of: 2026-09-12

- Finding: 48
  - Claim: MCP Registry publishing uses a `server.json` (schema `https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json`), the `mcp-publisher` CLI (`init`, `login github|dns|...`, `publish`), namespaces `io.github.<user>/*` (GitHub auth) or reverse-DNS `com.example/*` (DNS TXT auth), and package types npm, pypi, nuget, cargo, oci, and mcpb; a plain binary is published as `mcpb` pointing at a GitHub or GitLab release asset.
  - Detail: "Cargo packages use `"registryType": "cargo"` in `server.json`." "Rust MCP authors have two first-class distribution paths: Cargo (`registryType: cargo`), source-distributed via crates.io. End users need the Rust toolchain... MCPB (`registryType: mcpb`), prebuilt binary distributed via GitHub or GitLab Releases. End users need no toolchain." Ownership for cargo: "crates.io strips HTML comments during markdown to HTML conversion... Cargo authors must include the `mcp-name:` token as visible markdown text." Allowed registries: npm `https://registry.npmjs.org`, PyPI, NuGet, "Cargo: `https://crates.io` only", OCI (docker.io, ghcr.io, quay.io, `*.pkg.dev`, `*.azurecr.io`, mcr.microsoft.com), "MCPB: `https://github.com` releases and `https://gitlab.com` releases only." Org namespaces require GitHub org Owner role. The registry is "currently in preview." Example:
    ```json
    {
      "$schema": "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json",
      "name": "io.github.SHSharkar/pg-dba-mcp",
      "title": "PostgreSQL DBA",
      "description": "PostgreSQL DBA lifecycle tools over MCP, scoped to one database and schema.",
      "version": "0.1.0",
      "packages": [
        { "registryType": "cargo", "identifier": "pg-dba-mcp", "version": "0.1.0", "transport": { "type": "stdio" } },
        { "registryType": "mcpb", "identifier": "https://github.com/SHSharkar/pg-dba-mcp/releases/download/v0.1.0/pg-dba-mcp-darwin-arm64.mcpb", "version": "0.1.0", "transport": { "type": "stdio" } }
      ]
    }
    ```
  - Citations:
    - https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/package-types.mdx, Authoritative, retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/server-json/official-registry-requirements.md, Authoritative, retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/quickstart.mdx, Authoritative, retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/registry/releases (v1.8.1), Authoritative, 2026-08-06
    - https://github.com/modelcontextprotocol/registry/issues/1423, Consensus (closed issue), opened 2026-07-06
  - Confidence: Medium (80), see conflict C2 on production acceptance of `cargo`
  - As-of: 2026-09-12

- Finding: 49
  - Claim: macOS signing and notarization for a CLI: sign with a Developer ID Application certificate using `codesign --timestamp --options=runtime`, zip the binary, submit with `xcrun notarytool submit ... --wait`, and accept that a standalone binary cannot be stapled; ship it inside a signed `.pkg` or a notarized zip.
  - Detail: Apple: "Enable the Hardened Runtime capability for your app and command line targets." "`notarytool` submit works only with UDIF disk images, signed 'flat' installer packages, and zip files." "While you can notarize a ZIP archive, you can't staple to it directly... Although tickets are created for standalone binaries, it's not currently possible to staple tickets to them." "Starting November 1, 2023, the Apple notary service no longer accepts uploads from `altool`." Verification: `codesign -vvv --deep --strict`. cargo-dist handles Windows signing; macOS notarization remains a custom CI step.
  - Citations:
    - https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution, Authoritative, retrieved 2026-09-12
    - https://developer.apple.com/documentation/security/customizing-the-notarization-workflow, Authoritative, retrieved 2026-09-12
    - https://developer.apple.com/documentation/security/resolving-common-notarization-issues, Authoritative, retrieved 2026-09-12
  - Confidence: High (92)
  - As-of: 2026-09-12

- Finding: 50
  - Claim: SBOM and provenance: embed dependency data with `cargo auditable build`, emit CycloneDX with `cargo cyclonedx` (0.5.9) or SPDX with `cargo sbom` (0.10.0), and sign build provenance in GitHub Actions with `actions/attest-build-provenance` (SLSA v1.0 provenance predicate), verifiable with `gh attestation verify <file> -R owner/repo`; SLSA Build Level 3 needs a reusable workflow.
  - Detail: GitHub: "You can use GitHub Actions to generate artifact attestations that establish build provenance for artifacts such as binaries and container images." Verification "By default, this command enforces the `https://slsa.dev/provenance/v1` predicate type." SBOM attestation: "the `attest` action currently supports either SPDX or CycloneDX SBOM predicates." Availability: "artifact attestations are only available for public repositories" on Free, Pro, and Team plans. "Building software with reusable workflows and artifact attestations can streamline your supply chain security and help you achieve SLSA v1.0 Build Level 3." cargo-dist 0.33.0 fixed its `gh attestation` invocation, so attestations integrate with its pipeline.
  - Citations:
    - https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations, Authoritative, retrieved 2026-09-12
    - https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/increase-security-rating, Authoritative, retrieved 2026-09-12
    - https://github.com/actions/attest-build-provenance, Authoritative, retrieved 2026-09-12
    - https://cli.github.com/manual/gh_attestation_verify, Authoritative, retrieved 2026-09-12
    - https://crates.io/api/v1/crates/cargo-auditable, cargo-cyclonedx, cargo-sbom, Authoritative (registry API), read 2026-09-12
  - Confidence: High (90)
  - As-of: 2026-09-12

### Question 6: Testing and conformance

- Finding: 51
  - Claim: MCP Inspector 2.6.0 is one npm package with three clients (`--web` default, `--cli`, `--tui`), needs Node 22.19.0 or newer, launches stdio servers by positional command and HTTP servers by `--server-url <url> --transport http`, and its CLI runs one `--method` per invocation for CI.
  - Detail: "The Inspector requires Node 22.19.0 or newer and runs directly through `npx`." stdio: `npx @modelcontextprotocol/inspector --cli node build/index.js --method tools/list` (use `--` before server arguments). HTTP: `npx @modelcontextprotocol/inspector --cli https://my-mcp-server.example.com --transport http --method tools/list --header "X-API-Key: ..."`. Methods: `initialize` (connect-only probe), `tools/list`, `tools/call` with `--tool-name` and `--tool-arg` or `--tool-args-json`, `resources/list`, `resources/read --uri`, `resources/templates/list`, `prompts/list`, `prompts/get`, `logging/setLevel` ("Legacy era only"). Config file entries accept `"protocolEra": "modern"` and `"modernLogLevel"`. Docker: `docker run --rm --no-healthcheck ghcr.io/modelcontextprotocol/inspector --cli <target> --method tools/list`. Auth flags: `--use-stored-auth`, `--stored-auth-only`, `--relogin`, `--wait-for-auth`, `--print-handoff`. The Inspector does not yet publish a CIMD document, so CIMD auth must be tested with Claude Code or another CIMD client (Zuplo docs, May 2026).
  - Citations:
    - https://modelcontextprotocol.io/docs/2026-07-28/tools/inspector/cli, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/docs/2026-07-28/tools/inspector/configuration, Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/docs/2026-07-28/tools/inspector/recipes, Authoritative, retrieved 2026-09-12
    - https://github.com/modelcontextprotocol/inspector/blob/main/clients/cli/README.md, Authoritative, retrieved 2026-09-12
    - https://registry.npmjs.org/@modelcontextprotocol/inspector, Authoritative (registry API), 2.6.0 published 2026-09-09
    - https://zuplo.com/docs/articles/configuring-auth0-for-mcp-auth, Consensus, retrieved 2026-09-12
  - Confidence: High (93)
  - As-of: 2026-09-12

- Finding: 52
  - Claim: An official conformance suite exists: `npx @modelcontextprotocol/conformance server --url http://localhost:3000/mcp` runs all server scenarios against a Streamable HTTP server, validates every wire message against the spec JSON schema for the negotiated version, supports `--spec-version 2026-07-28`, `--requirements <revision>`, and an `--expected-failures` baseline; npm 0.1.16 (2026-03-30), repo pushed 2026-09-12.
  - Detail: "Every scenario also validates each JSON-RPC message on the wire against the spec's JSON schema for the negotiated spec version." Suites: `active` (default), `all`, `draft`, `pending`; client suites `core`, `extensions`, `backcompat`, `auth`, `metadata`, `draft`. "dated versions through `2025-11-25` use the stateful lifecycle (initialize handshake), while the 2026 draft (`2026-07-28`) uses the stateless lifecycle (per-request `_meta`)." Results land in `results/server-<scenario>-<timestamp>/checks.json`. Server testing takes a URL, so stdio conformance runs through the Inspector CLI or your own harness; the SDK integration guide in the repo shows how SDKs wire it into CI. Note the npm release (0.1.16) predates the final 2026-07-28 spec; the repo's README already documents 2026-07-28 support, so run from the repo or a newer tag if npm lags.
  - Citations:
    - https://github.com/modelcontextprotocol/conformance (README), Authoritative, pushed 2026-09-12, latest release v0.1.16 on 2026-03-27
    - https://registry.npmjs.org/@modelcontextprotocol/conformance, Authoritative (registry API), 0.1.16 published 2026-03-30
  - Confidence: High (88)
  - As-of: 2026-09-12

- Finding: 53
  - Claim: Integration tests against real PostgreSQL: use `testcontainers-modules` with the `postgres` feature (`Postgres::default().start().await`, `get_host_port_ipv4(5432)`, `with_init_sql`) locally and in CI, or a GitHub Actions `services:` container with the `postgres` image; `pgtemp` 0.7.1 (2025-11-10) is an embedded option that needs a local `initdb`.
  - Detail: testcontainers-modules docs: "Starts an instance of Postgres. This module is based on the official Postgres docker image. Default db name, user and password is `postgres`." Async: `use testcontainers_modules::{postgres, testcontainers::runners::AsyncRunner};`. Version alignment: "you don't need to explicitly depend on `testcontainers` as it's re-exported dependency of `testcontainers-modules` with aligned version." GitHub Docs: "Creating PostgreSQL service containers" shows `services: postgres: image: postgres` with `POSTGRES_PASSWORD` and health options. Community Postgres MCP servers run the read-only enforcement suite in CI "with a `postgres:16-alpine` service container" and fail the build if the integration tests report as skipped. Contract tests for tools: assert `tools/list` output (names, annotations, `outputSchema`) against golden JSON and validate `structuredContent` against the declared schema, which the conformance suite's wire-schema check complements.
  - Citations:
    - https://docs.rs/testcontainers-modules/latest/testcontainers_modules/postgres/struct.Postgres.html, Authoritative, retrieved 2026-09-12
    - https://rust.testcontainers.org/quickstart/community_modules/, Authoritative, retrieved 2026-09-12
    - https://crates.io/api/v1/crates/testcontainers (0.28.0, 2026-08-06) and https://crates.io/api/v1/crates/pgtemp (0.7.1, 2025-11-10), Authoritative (registry API), read 2026-09-12
    - https://docs.github.com/actions/guides/creating-postgresql-service-containers, Authoritative, retrieved 2026-09-12
    - https://github.com/samuel-cabral/safe-postgres-mcp, Consensus, retrieved 2026-09-12
  - Confidence: High (88)
  - As-of: 2026-09-12

- Finding: 54
  - Claim: Test both transports the way clients do: for stdio, spawn the binary and drive it with the Inspector CLI or rmcp's `TokioChildProcess` client; for HTTP, start the server on 127.0.0.1, run the conformance suite plus Inspector against `http://127.0.0.1:<port>/mcp`, and assert the 2026-07-28 specifics (400 with -32020 on header mismatch, 405 on GET or DELETE, `X-Accel-Buffering: no` on SSE, `ttlMs` and `cacheScope` on list results, deterministic `tools/list` order, and a legacy `initialize` path if you serve older clients).
  - Detail: rmcp README documents `TokioChildProcess` for launching a stdio server from a test client and `serve` with `(stdin(), stdout())` for the server side; its Stateless Streamable HTTP section describes `legacy_session_mode` for pre-2026 clients. The spec defines the exact status codes and headers to assert (Finding 16). The conformance README states the `2026-07-28` scenarios use the stateless lifecycle and that a server under test is addressed by URL.
  - Citations:
    - https://github.com/modelcontextprotocol/rust-sdk (README), Authoritative, retrieved 2026-09-12
    - https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http, Authoritative, 2026-07-28
    - https://github.com/modelcontextprotocol/conformance (README), Authoritative, retrieved 2026-09-12
  - Confidence: High (88)
  - As-of: 2026-09-12

## 3. Conflicts found and how they were resolved

- C1, cargo-dist version split: GitHub Releases (v0.33.0, published 2026-09-11T05:40:23Z) and npm `@axodotdev/dist` (0.33.0, 2026-09-11) disagree with crates.io (`cargo-dist` 0.32.0, 2026-05-22; the versions endpoint lists no 0.33.0, yanked or not). Resolution: the GitHub release is the project's primary distribution channel and its own installers point at it; report 0.33.0 as current and 0.32.0 as the crates.io ceiling for `cargo install cargo-dist`. Use the shell installer or `cargo binstall` to get 0.33.0.
- C2, registry cargo support: issue #1423 (2026-07-06) stated cargo publishes were rejected in production because no release included the merged support; the docs on `main` fully document `registryType: cargo`; releases v1.8.0 (2026-07-13) and v1.8.1 (2026-08-06) followed and the issue is closed. I could not find the word "cargo" in the v1.8.1 release body. Resolution: documented as supported at Medium confidence; verify with a real `mcp-publisher publish` before relying on it (Open question O1).
- C3, OWASP LLM edition: the brief names the 2025 list; OWASP published the 2026 edition v1.0 on 2026-08-03 with a different order. Resolution: primary PDF read; 2026 is current, 2025 is superseded. Numbering changed, so "LLM06 Excessive Agency" is now LLM03.
- C4, OWASP MCP Top 10 MCP06 title: the same official page lists MCP06 as "Intent Flow Subversion" in the summary and "Prompt Injection via Contextual Payloads" in the detailed list, with inconsistent zero-padding. Resolution: reported both titles; the project is v0.1 beta and says so.
- C5, client matrix: modelcontextprotocol.info and other mirrors show older matrices (for example Claude Code without Resources); the official page was deleted on 2026-05-27. Resolution: used the last upstream commit (87993a68, 2026-05-26) and labeled it historical; newer vendor docs override single cells (Claude Code now documents CIMD).
- C6, brief premises vs spec: the brief asks about Streamable HTTP session IDs, `Last-Event-ID` resumability, `logging/setLevel` levels, and server-initiated elicitation; 2026-07-28 removed or redesigned all four. Resolution: the changelog and transport page are authoritative; findings describe the current mechanisms and the legacy compatibility path.
- C7, tool annotation effects in Claude: the Agent SDK docs say destructiveHint, idempotentHint, and openWorldHint are "Informational only" and readOnlyHint only governs parallel execution, while connector review criteria say readOnlyHint skips per-call confirmation and destructiveHint always prompts. Resolution: these describe different surfaces (in-process SDK tools vs remote connectors in claude.ai and Claude Desktop); both reported in Finding 6.
- C8, mcp-remote callback port: older README text says default port 3334; the 0.13.5 README says the port is derived from the server URL in 3335 to 49150. Resolution: the npm registry README for the latest version wins.
- C9, Windsurf docs: docs.windsurf.com redirects to docs.devin.ai; third-party guides describe `serverUrl` as the only remote field, while the official page accepts `serverUrl` or `url`. Resolution: official page wins; both fields reported.
- C10, Keycloak: vendor marketing and community posts describe Keycloak as MCP-ready; Keycloak's own conformance table says RFC 8707 is "Not supported" and CIMD is experimental. Resolution: the vendor's own table wins.
- C11, claude.ai authless servers: the auth docs list `none` as supported, while issue anthropics/claude-ai-mcp#402 (2026-06-04) shows an org-managed custom connector failing on an unauthenticated server; the maintainer reply attributed that case to a wrong URL. Resolution: `none` is documented as supported; flagged as Open question O4 because the failure mode is easy to hit.
- C12, MCPB manifest version: MANIFEST.md examples use `"manifest_version": "0.3"` while the schemas directory includes v0.4 and Anthropic's plugin skill instructs `"0.4"`. Resolution: v0.4 is the newest schema file in the repo; examples lag.

## 4. Open questions (below 50 percent confidence)

- O1: Does registry.modelcontextprotocol.io accept `registryType: cargo` in production today? Searched: registry issues, release notes v1.8.0 and v1.8.1, package-types doc. Docs say yes; the only production evidence is an issue closed after a release. A test publish is the only way to know.
- O2: Will crates.io receive cargo-dist 0.33.0, or is crates.io publishing now optional for the project? Searched: crates.io versions endpoint, release notes. No statement found.
- O3: When will Keycloak add RFC 8707 resource indicators, and when will Ory Hydra ship CIMD? Searched: Keycloak MCP page, Keycloak release notes 26.6 and 26.7, ory/hydra#4061 (open, 2026-09-09). No dates.
- O4: Does a claude.ai custom connector reliably connect to a server with no authorization metadata (`none` type) for individual users, given issue #402? Searched: claude.com auth docs, claude-ai-mcp issues. Docs say supported; one field report disagrees.
- O5: Does Claude Desktop skip confirmation for `readOnlyHint: true` tools served by local stdio servers, as it does for connectors? Searched: Claude Desktop help center, connectors review criteria, MCP annotations blog. No Desktop-specific statement found.
- O6: What exact truncation Claude Code applies to tool descriptions and server instructions (a 2KB cut is reported by a secondary source only). Searched: code.claude.com mcp and tool-search pages. No primary statement found.

## 5. Methodology

- Searches run per tool:
  - Exa: 37
  - Serper: 40
  - Gap between Exa and Serper: 3 searches, 7.5 percent of the larger count, within the 10 percent rule
  - Tavily: 6 (floor 5, plus 1 expansion to cross-check the OWASP 2026 edition conflict C3)
  - Brave: 5
  - Built-in WebSearch: 5
  - Serper specialized indexes: none used
- Fetches:
  - Local reader `mcp__read-website__read_website`: 17 calls. 13 returned full main content. 2 refused `text/plain` (raw GitHub files) and were re-read with curl. 2 redirected: `/clients` to the intro page (which exposed the removal) and docs.windsurf.com to docs.devin.ai (content valid).
  - Paid fetchers (`mcp__exa__web_fetch_exa`, `mcp__serper__webpage_scrape`, `mcp__tavily__tavily_extract`): 0 calls needed.
  - Registry and repository API reads (curl and authenticated `gh`): crates.io 13 (12 crates plus the cargo-dist versions list), npm 7, GitHub releases and repo metadata 14, raw GitHub files 9 (package-types.mdx, official-registry-requirements.md, rust-sdk README, conformance README, historical clients.mdx, extensions client-matrix.mdx, docs.json, MANIFEST.md, mcpb schemas listing), GitHub tree and commit queries 4, OWASP 2026 PDF 1 (pdftotext), Claude Code CIMD document 1, HTTP HEAD checks 2. Anonymous GitHub API hit a rate limit once; the same calls were repeated with the authenticated `gh` CLI (not a dead tool).
  - Persisted Exa outputs read from disk: 3.
- Primary sources fetched and read in full: MCP changelog 2026-07-28; Streamable HTTP transport 2026-07-28; Tools 2026-07-28; Security Best Practices 2026-07-28; Homebrew tap guide; PostgreSQL 18 client connection defaults; OWASP MCP Top 10 project page; OWASP GenAI LLM Top 10 2026 page and PDF table of contents; MCP extensions client matrix (page and source); Claude connector review criteria; Claude MCP tunnels overview; Cloudflare Agents authorization page; Devin (Windsurf) Cascade MCP page; registry issue #1423; registry package-types.mdx and official-registry-requirements.md; rust-sdk README (intro, stateless HTTP, features); conformance README; historical clients.mdx at 87993a68; mcpb MANIFEST.md and schemas listing; Claude Code CIMD document.
- Sources evaluated versus selected: about 210 distinct URLs surfaced across searches; 96 cited. Discarded as false positives or leads only: modelcontextprotocol.info mirror (stale matrix, not official), deepwiki pages (AI-generated summaries), Medium and dev.to posts (used only as leads, claims re-verified upstream), SEO guides (startdebugging.net, agentnotebook.dev, nxcode.io, usecarly.com, explainx.ai, fast.io, evomap.ai, policylayer.com, connector.zone), NHIMG editorial, obot.ai and towardsdatascience security guides (secondary restatements of the spec), openai-codex.mintlify.app (unofficial mirror), and the 2025 MCP spec pages where the 2026-07-28 page superseded them.
- Depth honored: the brief specified the per-question floors above; all were met, and Exa and Serper were expanded together past the floors until the twelve conflicts above were resolved or flagged.

## 6. Outages

- None. No search or fetch tool was marked dead. The only failures were two `text/plain` refusals by the local reader (worked around with curl) and one anonymous GitHub API rate limit (worked around with the authenticated `gh` CLI).

## 7. Limitations

- Em dashes inside quoted source text were replaced with commas or ellipses to comply with the output rules; the wording is otherwise verbatim.
- Living documentation pages (code.claude.com, claude.com/docs, cursor.com, code.visualstudio.com, developers.openai.com, zed.dev, docs.devin.ai, docs.rs, modelcontextprotocol.io docs) carry no publication date; they are cited as retrieved on 2026-09-12 with in-page version markers where present. Re-check them before shipping.
- The client capability matrix is a historical snapshot from 2026-05-26 because the official page was removed; individual cells may be stale, and the community `mcp-client-capabilities` package was not independently verified cell by cell.
- The Anthropic "Code execution with MCP" post shows no date on the page; it was used only as supporting context for the `search_tools` pattern.
- The claude.ai egress range `160.79.104.0/21` and the `Claude-User` user agent are from official docs and a community post respectively; the user agent string was not confirmed in an official page.
- No page contained instructions aimed at this agent; nothing was ignored on that basis.
- Items needing the Principal Architect's verification: O1 (test publish with `registryType: cargo`), O4 (connect an authless test server as a claude.ai custom connector), and O5 (observe Claude Desktop's confirmation behavior for a local `readOnlyHint: true` tool).
