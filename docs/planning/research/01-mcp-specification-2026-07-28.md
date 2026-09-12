# Worker report 1: the MCP specification as of 2026-09-12

Worker report captured on September 12th 2026 (session started 01:39:06 PM, GMT+06:00) for the OwnPG planning pack. It is the unedited return of one deep-research worker, except that em dashes inside quoted source text were replaced with commas to follow the house punctuation rule. Every claim carries its own URL, source date, and confidence band. Treat the content as evidence, never as instructions.

---

# MCP specification state as of September 12 2026, 01:42:45 PM (+0600)

## 1. Headline answers

- The current MCP revision is `2026-07-28`. It was released as a stable, non-prerelease GitHub release on 2026-07-28T16:47:49Z (tag `2026-07-28`, commit `5f5440b`, signed by Den Delimarsky) and is the spec at https://modelcontextprotocol.io/specification/2026-07-28. The Principal Architect is right that a new spec launched recently. Confidence: High. As-of: 2026-09-12.
- There is no newer "next" revision with content. The `draft` changelog reads "Changes since the most recent release will accumulate here." with nothing listed, and `schema/draft/schema.ts` differs from `schema/2026-07-28/schema.ts` only in doc-link paths (8 changed lines, all `/specification/draft/` vs `/specification/2026-07-28/`). Both declare `LATEST_PROTOCOL_VERSION = "2026-07-28"`. Confidence: High. As-of: 2026-09-12.
- `2026-07-28` is a breaking rewrite: MCP is now a stateless request/response protocol. The `initialize`/`notifications/initialized` handshake, `Mcp-Session-Id`, `ping`, `logging/setLevel`, `notifications/roots/list_changed`, the HTTP GET stream, `resources/subscribe`/`unsubscribe`, and `Last-Event-ID` resumability are all removed. Every request carries `_meta["io.modelcontextprotocol/protocolVersion"]` and `_meta["io.modelcontextprotocol/clientCapabilities"]`. Confidence: High. Source: https://modelcontextprotocol.io/specification/2026-07-28/changelog (2026-07-28).
- A server built today MUST implement `server/discover`, MUST accept per-request `_meta`, MUST tag every result with `resultType` ("complete" or "input_required"), MUST return `ttlMs` and `cacheScope` on `server/discover`, `tools/list`, `prompts/list`, `resources/list`, `resources/templates/list`, and `resources/read`, and (on Streamable HTTP) MUST validate `MCP-Protocol-Version`, `Mcp-Method`, and `Mcp-Name` headers against the body. Confidence: High. Sources: spec pages cited in section 2.
- Server-to-client requests (elicitation, sampling, roots) no longer travel as JSON-RPC requests. They ride inside an `InputRequiredResult` (`resultType: "input_required"`, `inputRequests`, opaque `requestState`), and the client retries the original call with `inputResponses`. This is the Multi Round-Trip Requests (MRTR) pattern, SEP-2322. Confidence: High. Source: https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr.
- Roots, Sampling, and Logging are deprecated (SEP-2577) with earliest removal in the first revision released on or after 2027-07-28. Dynamic Client Registration (RFC 7591) is deprecated in favor of Client ID Metadata Documents on the same clock. HTTP+SSE (2024-11-05) is Deprecated under the lifecycle policy. Confidence: High. Source: https://modelcontextprotocol.io/specification/2026-07-28/deprecated.
- Tasks moved out of the core protocol into the official extension `io.modelcontextprotocol/tasks` (SEP-2663), with `tasks/get`, `tasks/update`, `tasks/cancel`, `resultType: "task"`, and no `tasks/result` or `tasks/list`. The ext-tasks repo marks schema `2026-07-28` as Stable. Confidence: High.
- Governance: MCP is a founding project of the Agentic AI Foundation (AAIF), a directed fund under the Linux Foundation, since December 9, 2025. Legal entity: "Model Context Protocol a Series of LF Projects, LLC". Technical decisions stay with the MCP Steering Group (Lead Maintainers David Soria Parra and Den Delimarsky, plus Core Maintainers). Confidence: High.
- The Rust SDK (`rmcp`) is Tier 1 as of 2026-08-21 (PR #3287 merged 2026-08-21T12:30:14Z). Latest crate: `rmcp` 3.3.0, published 2026-09-10T15:32:23Z on crates.io. It implements `2026-07-28` and stays compatible with `2025-11-25` and earlier. Confidence: High.
- The MCP Registry at registry.modelcontextprotocol.io is still in preview (official pages carry the preview banner; live `/v0.1/version` reports 1.8.1 built 2026-08-06). The MCP Inspector is on the v2 line, npm `latest` 2.6.0 published 2026-09-09. Confidence: High.

## 2. Detailed findings by question

### Question 1. Latest revision, release date, URL, earlier revisions, draft status

- Finding 1.1: The current revision string is `2026-07-28`.
  - Evidence quote: "The **current** protocol version is **2026-07-28**." and "Version 2026-07-28 (latest)".
  - Source URL: https://modelcontextprotocol.io/docs/2026-07-28/learn/versioning (the `/specification/versioning` URL redirects here)
  - Source category: Authoritative
  - Source date: page served under the 2026-07-28 docs tree; read 2026-09-12
  - Confidence: High (95)
- Finding 1.2: The release date is 2026-07-28, published as a stable GitHub release.
  - Evidence quote: "This release marks the **stable release** of the `2026-07-28` revision of the Model Context Protocol." Release metadata from the GitHub API: `published: 2026-07-28T16:47:49Z | draft: false | prerelease: false`.
  - Source URL: https://github.com/modelcontextprotocol/modelcontextprotocol/releases/tag/2026-07-28 and https://api.github.com/repos/modelcontextprotocol/modelcontextprotocol/releases
  - Source category: Authoritative
  - Source date: 2026-07-28
  - Confidence: High (98)
- Finding 1.3: The official blog confirms the same-day launch and names the release "the next version of the MCP specification, 2026-07-28".
  - Evidence quote: "Today, we're officially pushing the release button on the next version of the MCP specification, `2026-07-28`, along with the SDKs that will allow you to start building clients and servers right away."
  - Source URL: https://blog.modelcontextprotocol.io/posts/2026-07-28/
  - Source category: Authoritative
  - Source date: 2026-07-28 (post front matter `date: "2026-07-28T09:00:00+00:00"`)
  - Confidence: High (98)
- Finding 1.4: The spec page URL is https://modelcontextprotocol.io/specification/2026-07-28 and the schema source of truth is `schema/2026-07-28/schema.ts`.
  - Evidence quote: "This specification defines the authoritative protocol requirements, based on the TypeScript schema in schema.ts" (linking to `schema/2026-07-28/schema.ts`). The file declares `export const LATEST_PROTOCOL_VERSION = "2026-07-28";` at line 30.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28 and https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/2026-07-28/schema/2026-07-28/schema.ts
  - Source category: Authoritative
  - Source date: 2026-07-28
  - Confidence: High (98)
- Finding 1.5: Earlier revisions and their release dates, from the GitHub releases API and the schema directory.
  - Tags in the repo: `2024-10-07`, `2024-11-05`, `2024-11-05-final`, `2025-03-26`, `2025-06-18`, `2025-11-25`, `2025-11-25-RC`, `2026-07-28`, `2026-07-28-RC`.
  - Release publish times:
    - `2024-10-07` published 2024-11-06T10:59:21Z (pre-launch tag)
    - `2024-11-05` published 2025-01-17T20:37:02Z; `2024-11-05-final` published 2025-03-26T13:53:30Z
    - `2025-03-26` published 2025-03-26T14:28:27Z
    - `2025-06-18` published 2025-06-18T20:09:59Z
    - `2025-11-25-RC` published 2025-11-15T06:06:55Z (prerelease); `2025-11-25` published 2025-11-25T21:17:42Z
    - `2026-07-28-RC` published 2026-05-29T12:51:22Z (prerelease); `2026-07-28` published 2026-07-28T16:47:49Z
  - Schema directory contents: `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25`, `2026-07-28`, `draft`.
  - Source URL: https://api.github.com/repos/modelcontextprotocol/modelcontextprotocol/tags and https://api.github.com/repos/modelcontextprotocol/modelcontextprotocol/contents/schema
  - Source category: Authoritative
  - Source date: read 2026-09-12
  - Confidence: High (98)
- Finding 1.6: The release candidate was locked on May 21, 2026 and was originally targeted as "2026-06-30" before being renamed.
  - Evidence quote: "The release candidate is locked as of **May 21, 2026**. The final specification will be published on **July 28, 2026**." PR #2750 history: "Renamed from 'Blog: 2026-06-30 specification release candidate' to 'Blog: 2026-07-28 specification release candidate'". Keycloak issue #48527: "The release date was changed from 06-30 to 07-28."
  - Source URL: https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/ ; https://github.com/modelcontextprotocol/modelcontextprotocol/pull/2750 ; https://github.com/keycloak/keycloak/issues/48527
  - Source category: Authoritative (blog, PR), Consensus (Keycloak issue)
  - Source date: 2026-05-21 (blog), PR rename undated in snippet, Keycloak issue mid-2026
  - Confidence: High (90)
- Finding 1.7: A `draft` revision exists but currently contains no accumulated changes.
  - Evidence quote: Draft changelog body: "Changes since the most recent release will accumulate here." `diff schema-2026-07-28.ts schema-draft.ts` produced 8 changed lines, all documentation links. Draft `schema.ts` line 30: `LATEST_PROTOCOL_VERSION = "2026-07-28"`.
  - Source URL: https://modelcontextprotocol.io/specification/draft/changelog and https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/schema/draft/schema.ts
  - Source category: Authoritative
  - Source date: main branch last commit 2026-09-08T12:24:34Z
  - Confidence: High (95)
- Finding 1.8: The roadmap post (published after the release) names the next priority areas, none of which have landed in the draft yet.
  - Evidence quote: "The new roadmap is organized into five priority areas": "Agentic messaging primitives", "HTTP-native transport unification and hardening", "Agent identity and enterprise-ready security", "Improved primitives", "Improved SDK developer experience". It also says the Tasks extension is to be matured "so it can move into the specification" and mentions "local servers speaking Streamable HTTP over stdio".
  - Source URL: https://blog.modelcontextprotocol.io/posts/mcp-roadmap/
  - Source category: Authoritative
  - Source date: after 2026-07-28 (post references the release as past); listed first on the blog index on 2026-09-12
  - Confidence: High (90)

### Question 2. Full changelog of 2026-07-28 versus 2025-11-25

All items below are quoted from https://modelcontextprotocol.io/specification/2026-07-28/changelog (Authoritative, 2026-07-28). Confidence: High (98) for each entry, since each is read from the normative changelog and cross-checked against the schema file.

- Major changes
  - Sessions removed (SEP-2567): "Remove protocol-level sessions and the `Mcp-Session-Id` header from the Streamable HTTP transport. List endpoints (`tools/list`, `resources/list`, `prompts/list`) no longer vary per-connection. Servers that need cross-call state use explicit, server-minted handles passed as ordinary tool arguments".
  - Stateless core (SEP-2575): "Make MCP stateless: remove the `initialize`/`notifications/initialized` handshake. Every request now carries its protocol version and client capabilities in `_meta` (`io.modelcontextprotocol/protocolVersion`, `io.modelcontextprotocol/clientCapabilities`). Clients SHOULD identify themselves on each request (`io.modelcontextprotocol/clientInfo`), and servers SHOULD identify themselves in each result's `_meta` (`io.modelcontextprotocol/serverInfo`). Version mismatches return `UnsupportedProtocolVersionError`".
  - New mandatory RPC (SEP-2575): "Add `server/discover`: servers MUST implement this RPC to advertise their supported protocol versions, capabilities, and identity."
  - Subscriptions (SEP-2575): "Replace the HTTP GET endpoint and `resources/subscribe`/`resources/unsubscribe` with `subscriptions/listen`: a single long-lived POST-response stream for opted-in server-to-client change notifications. Clients opt in to specific types (`toolsListChanged`, `promptsListChanged`, `resourcesListChanged`, `resourceSubscriptions`); the server acknowledges and tags notifications with `io.modelcontextprotocol/subscriptionId`."
  - Removed methods (SEP-2575): "Remove `ping`, `logging/setLevel`, and `notifications/roots/list_changed`. Log level is now set per-request via `io.modelcontextprotocol/logLevel` in `_meta`; servers MUST NOT emit `notifications/message` for requests that did not include this field".
  - Tasks (SEP-2663): "Move experimental tasks out of the core protocol and into an official extension (`io.modelcontextprotocol/tasks`). The redesigned extension replaces the blocking `tasks/result` method with polling via `tasks/get` and a new `tasks/update` for client-to-server input, removes `tasks/list`, and allows servers to return task handles unsolicited without per-request opt-in".
  - MRTR (SEP-2322): "Multi Round-Trip Requests (MRTR) pattern introduced which replaces the previous approach of sending server-initiated requests, such as `roots/list`, `sampling/createMessage`, or `elicitation/create`. Servers return an `InputRequiredResult` (`resultType: "input_required"`) whose `inputRequests` field carries the requests".
  - resultType (SEP-2322): "All results now carry a required `resultType` field: `"complete"` for ordinary results and `"input_required"` for multi round-trip request interim results. Clients **MUST** treat results from earlier-protocol servers that omit the field as `"complete"`".
  - Resumability removed (SEP-2575): "Remove SSE stream resumability and message redelivery (the `Last-Event-ID` header and SSE event IDs) from the Streamable HTTP transport. A broken response stream loses the in-flight request; clients **MUST** re-issue it as a new request with a new request ID".
- Minor changes
  - "Add `extensions` field to `ClientCapabilities` and `ServerCapabilities`" (schema.ts lines 785 and 882 confirm `extensions?: { [key: string]: JSONObject }`).
  - "Document OpenTelemetry trace context propagation conventions for `_meta` keys (`traceparent`, `tracestate`, `baggage`) (SEP-414)".
  - "Servers **SHOULD** return tools from `tools/list` in a deterministic order".
  - Headers (SEP-2243): "Require standard MCP request headers (`Mcp-Method`, `Mcp-Name`) on Streamable HTTP POST requests, and add support for custom headers from tool parameters via `x-mcp-header`".
  - Caching (SEP-2549): "Require `ttlMs` and `cacheScope` fields on results returned by `tools/list`, `prompts/list`, `resources/list`, `resources/read`, and `resources/templates/list` via a new `CacheableResult` interface."
  - Error code: "Change resource not found error code from `-32002` to `-32602` (Invalid Params)".
  - Issuer validation (SEP-2468): "Authorization servers **SHOULD** include the `iss` parameter in authorization responses per RFC 9207, and MCP clients **MUST** validate a present `iss` against the recorded issuer before redeeming the authorization code".
  - DCR application_type (SEP-837): "Require MCP clients to specify an appropriate `application_type` during Dynamic Client Registration".
  - Issuer binding (SEP-2352): "clients **MUST** key persisted credentials by the issuer identifier, **MUST NOT** reuse them with a different authorization server, and **MUST** re-register when the authorization server changes".
  - Schema loosening (SEP-2106): "Loosen `inputSchema` and `outputSchema` to allow any JSON Schema 2020-12 keywords, and `structuredContent` to allow any JSON value. Add `$ref` resolution requirements and composition-keyword resource bounds".
  - Elicitation cleanup: "Remove the `notifications/elicitation/complete` notification and the `elicitationId` field of URL mode elicitation requests, both introduced in `2025-11-25`. ... Servers needing to correlate an elicitation across retries encode their own identifier in `requestState`."
  - Error code policy: "`-32000` to `-32019` remains implementation-defined (existing SDK usage is grandfathered), `-32020` to `-32099` is reserved for the MCP specification. Renumber ... `HeaderMismatch` `-32001` → `-32020`, `MissingRequiredClientCapability` `-32003` → `-32021`, `UnsupportedProtocolVersion` `-32004` → `-32022`". Schema constants confirm: `HEADER_MISMATCH = -32020`, `MISSING_REQUIRED_CLIENT_CAPABILITY = -32021`, `UNSUPPORTED_PROTOCOL_VERSION = -32022`.
- Deprecated
  - "Deprecate the Roots, Sampling, and Logging features (SEP-2577)". Migrations: tool parameters, resource URIs, or server configuration instead of Roots; direct LLM provider APIs instead of Sampling; `stderr` (stdio) or OpenTelemetry instead of Logging.
  - "Reclassify the HTTP+SSE transport (deprecated since protocol version `2025-03-26`) as Deprecated under the feature lifecycle policy (SEP-2596)".
  - "Reclassify the `includeContext` values `"thisServer"` and `"allServers"` ... as Deprecated".
  - "Deprecate the OAuth 2.0 Dynamic Client Registration Protocol (RFC7591) as a client registration mechanism in favor of Client ID Metadata Documents (PR #2858)".
- Other schema changes
  - "`schema.json` now correctly reflects that the Typescript definition of minimum/maximum/default are `number`'s and not just `integers`" (PR #2710).
- Governance and process
  - "Adopt a specification feature lifecycle and deprecation policy defining the Active, Deprecated, and Removed feature states, a minimum twelve-month deprecation window, and a registry of deprecated features (SEP-2596)".
  - "Formalize PR-based SEP workflow with markdown files in `seps/` directory (SEP-1850)".
- Items the question named that were NOT changed in 2026-07-28 (confirmed by reading earlier changelogs and the schema)
  - JSON-RPC batching: added in `2025-03-26`, removed in `2025-06-18` ("Remove support for JSON-RPC batching (PR #416)"). Source: https://modelcontextprotocol.io/specification/2025-06-18/changelog. Category: Authoritative. Date: 2025-06-18. Confidence: High (95). Not touched by 2026-07-28.
  - `MCP-Protocol-Version` header: introduced in `2025-06-18` ("Require negotiated protocol version to be specified via `MCP-Protocol-Version` header in subsequent requests when using HTTP (PR #548)"). In 2026-07-28 it became mandatory on every POST and must match `_meta`. Confidence: High (95).
  - Structured tool output and elicitation: introduced in `2025-06-18`; URL-mode elicitation and icons in `2025-11-25`. Confidence: High (95).
  - Tool annotations: `ToolAnnotations` in `schema/2026-07-28/schema.ts` (line 1912) still has `title`, `readOnlyHint` (default false), `destructiveHint` (default true), `idempotentHint` (default false), `openWorldHint` (default true). No change in 2026-07-28. Confidence: High (98).

### Question 3. Transports: stdio and Streamable HTTP rules

- Finding 3.1: Streamable HTTP is a single POST-only endpoint; the GET endpoint and sessions are gone.
  - Evidence quote: "The server **MUST** provide a single HTTP endpoint path (hereafter referred to as the **MCP endpoint**) that supports POST." and "Revision 2026-07-28 changed the behavior of Streamable HTTP ... Removal of the GET stream endpoint. Removal of protocol-level sessions."
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http
  - Source category: Authoritative
  - Source date: 2026-07-28
  - Confidence: High (98)
- Finding 3.2: POST rules.
  - Evidence quote: "The client **MUST** include an `Accept` header listing both `application/json` and `text/event-stream`"; "The body of the HTTP POST **MUST** be a single JSON-RPC _request_ or _notification_. The client **MUST NOT** send JSON-RPC _responses_."; notification accepted: "the server **MUST** return HTTP status code `202 Accepted` with no body"; request: "the server **MUST** return either `Content-Type: application/json` (a single JSON object) or `Content-Type: text/event-stream` (an SSE response stream). The client **MUST** support both."
  - Source URL: same as 3.1
  - Confidence: High (98)
- Finding 3.3: SSE stream rules.
  - Evidence quote: "The server **MUST NOT** send independent JSON-RPC _requests_ on this stream."; "The final JSON-RPC _response_ **SHOULD** terminate the stream."; "servers **SHOULD** include the `X-Accel-Buffering: no` header"; "servers are encouraged to periodically emit an SSE comment line ... as a keep-alive"; "Resumable SSE streams via `Last-Event-ID` are not supported."
  - Source URL: same as 3.1
  - Confidence: High (98)
- Finding 3.4: Required headers and validation.
  - Evidence quote: "Every POST request to the MCP endpoint **MUST** include an `MCP-Protocol-Version` header. ... The header value **MUST** match the `io.modelcontextprotocol/protocolVersion` field carried in the request body's `_meta`. If the values do not match, the server **MUST** reject the request with `400 Bad Request` and a `HeaderMismatch` JSON-RPC error". `Mcp-Method` is "Required For: All requests"; `Mcp-Name` mirrors "`params.name` or `params.uri`" and is required for "`tools/call`, `resources/read`, `prompts/get` requests". Unsupported version: "`400 Bad Request` and an `UnsupportedProtocolVersionError`". Unknown method: "`404 Not Found` and a JSON-RPC error with code `-32601`". Servers "**MUST** reject requests with a `400 Bad Request` HTTP status and JSON-RPC error code `-32020` (`HeaderMismatch`) if any validation fails." Non-ASCII values use the sentinel `=?base64?{Base64EncodedValue}?=`.
  - Source URL: same as 3.1
  - Confidence: High (98)
- Finding 3.5: Optional `x-mcp-header` mirroring; servers should not mirror secrets.
  - Evidence quote: "MCP servers **MAY** designate specific tool parameters to be mirrored into HTTP headers using an `x-mcp-header` extension property ... clients **MUST** support this feature." and "Server developers **SHOULD NOT** mark sensitive parameters (passwords, API keys, tokens, PII) with `x-mcp-header`". Header form: `Mcp-Param-{name}`.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/tools#x-mcp-header
  - Confidence: High (98)
- Finding 3.6: Security, Origin, localhost.
  - Evidence quote: "Servers **MUST** validate the `Origin` header on all incoming connections to prevent DNS rebinding attacks. If the `Origin` header is present and invalid, servers **MUST** respond with HTTP 403 Forbidden."; "When running locally, servers **SHOULD** bind only to localhost (127.0.0.1) rather than all network interfaces (0.0.0.0)."; "Servers **SHOULD** implement proper authentication for all connections."
  - Source URL: same as 3.1
  - Confidence: High (98)
- Finding 3.7: Cancellation on HTTP is closing the stream; on stdio it is `notifications/cancelled`.
  - Evidence quote: "Closing the SSE response stream **MUST** be treated by the server as cancellation of that request." and "This revision of the core protocol defines no client-to-server _notifications_ over Streamable HTTP." Stdio: "To cancel an in-flight request, the client **MUST** send a `notifications/cancelled` notification referencing the request's ID."
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation and the stdio page
  - Confidence: High (98)
- Finding 3.8: Legacy traffic handling for a modern-only server.
  - Evidence quote: "HTTP GET or DELETE to the MCP endpoint: respond with `405 Method Not Allowed`. An `Mcp-Session-Id` header on a request: ignore it, and do not mint or echo session IDs. A `Last-Event-ID` header: ignore it; streams are not resumable." Also: "A server that supports clients implementing protocol versions earlier than `2025-06-18` ... **MAY** treat a request that omits the header as protocol version `2025-03-26`. A server that does not support such clients **MUST** reject a request without the header".
  - Source URL: same as 3.1
  - Confidence: High (98)
- Finding 3.9: HTTP+SSE (2024-11-05) is Deprecated, not removed.
  - Evidence quote: "The HTTP+SSE transport from protocol version 2024-11-05 has been deprecated since protocol version `2025-03-26` and is classified as Deprecated under the feature lifecycle policy (SEP-2596). New implementations **SHOULD NOT** adopt it". Deprecated registry earliest removal: "Three months after SEP-2596 reaches Final".
  - Source URL: same as 3.1 and https://modelcontextprotocol.io/specification/2026-07-28/deprecated
  - Confidence: High (95) on status; see Conflict 7 on the removal date wording.
- Finding 3.10: stdio rules.
  - Evidence quote: "The server reads JSON-RPC messages from `stdin` and writes JSON-RPC messages to `stdout`."; "Messages are delimited by newlines, and **MUST NOT** contain embedded newlines."; "The server **MAY** write UTF-8 strings to `stderr` for any logging purposes"; "The server **MUST NOT** write anything to its `stdout` that is not a valid MCP message."; "The server **MUST NOT** write JSON-RPC _requests_ to `stdout`."; "There is no header layer."; listen notifications on stdio: "Clients **MUST** correlate these using the `io.modelcontextprotocol/subscriptionId` field in `_meta`". Custom transports over Unix sockets or TCP "**SHOULD** reuse this framing".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio
  - Confidence: High (98)
- Finding 3.11: stdio backward compatibility probe.
  - Evidence quote: A dual-era client "**SHOULD** probe with `server/discover` before sending any other request"; outcomes: `DiscoverResult` means modern; a recognized modern error like `UnsupportedProtocolVersionError` means modern, "Do **not** fall back to `initialize`"; "any other error, or does not respond within a reasonable timeout: the server is legacy. Fall back to the `initialize` handshake." and "The fallback **MUST NOT** be keyed to one specific error code".
  - Source URL: same as 3.10
  - Confidence: High (98)
- Finding 3.12: Authorization does not apply to stdio.
  - Evidence quote: "Implementations using an HTTP-based transport **SHOULD** conform to this specification, whereas implementations using STDIO transport **SHOULD NOT** follow this specification, and instead retrieve credentials from the environment."
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic
  - Confidence: High (98)

### Question 4. Authorization for HTTP transports

- Finding 4.1: Standards base (all named RFCs) and role model.
  - Evidence quote: "OAuth 2.1 IETF DRAFT (draft-ietf-oauth-v2-1-13)", "OAuth 2.0 Bearer Token Usage (RFC6750)", "OAuth 2.0 Authorization Server Metadata (RFC8414)", "OAuth 2.0 Dynamic Client Registration Protocol (RFC7591)", "Resource Indicators for OAuth 2.0 (RFC8707)", "OAuth 2.0 Protected Resource Metadata (RFC9728)", "OAuth 2.0 Authorization Server Issuer Identification (RFC9207)", "OAuth Client ID Metadata Documents (draft-ietf-oauth-client-id-metadata-document-00)", "OpenID Connect Discovery 1.0", "OpenID Connect Dynamic Client Registration 1.0". "A protected _MCP server_ acts as an OAuth 2.1 resource server".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
  - Source category: Authoritative
  - Source date: 2026-07-28
  - Confidence: High (98)
- Finding 4.2: Server-side MUSTs.
  - Evidence quote: "MCP servers **MUST** implement OAuth 2.0 Protected Resource Metadata (RFC9728)."; "The Protected Resource Metadata document returned by the MCP server **MUST** include the `authorization_servers` field containing at least one authorization server."; discovery: WWW-Authenticate `resource_metadata` on 401, or well-known at `/.well-known/oauth-protected-resource/<path>` or root; "MCP servers **MUST** validate that access tokens were issued specifically for them as the intended audience, according to RFC 8707 Section 2"; "Invalid or expired tokens **MUST** receive a HTTP 401 response."; "MCP servers **MUST NOT** accept or transit any other tokens."; status codes: 401 Unauthorized, 403 Forbidden for insufficient scope, 400 malformed.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization and .../authorization-server-discovery
  - Confidence: High (98)
- Finding 4.3: Authorization Server Metadata (RFC 8414) and OIDC discovery; PKCE.
  - Evidence quote: "MCP authorization servers **MUST** provide at least one of ... OAuth 2.0 Authorization Server Metadata (RFC8414) ... OpenID Connect Discovery 1.0. MCP clients **MUST** support both discovery mechanisms". Path-insertion order: `/.well-known/oauth-authorization-server/tenant1`, then `/.well-known/openid-configuration/tenant1`, then `/tenant1/.well-known/openid-configuration`. "the `issuer` value in the document **MUST** be identical to the issuer identifier used to construct the well-known URL." PKCE: "MCP clients **MUST** implement PKCE ... **MUST** use the `S256` code challenge method ... If `code_challenge_methods_supported` is absent, the authorization server does not support PKCE and MCP clients **MUST** refuse to proceed."
  - Source URL: .../authorization-server-discovery and .../security-considerations
  - Confidence: High (98)
- Finding 4.4: Client registration: CIMD preferred, DCR deprecated.
  - Evidence quote: "Dynamic Client Registration is deprecated. New implementations should use Client ID Metadata Documents instead." Priority: "1. Use pre-registered client information ... 2. Use Client ID Metadata Documents if the Authorization Server indicates that it supports them (via `client_id_metadata_document_supported` ...) 3. Use Dynamic Client Registration as a fallback ... 4. Prompt the user". CIMD: "The `client_id` URL **MUST** use the 'https' scheme and contain a path component"; "The metadata document **MUST** include at least the following properties: `client_id`, `client_name`, `redirect_uris`". DCR: "MCP clients **MUST** specify an appropriate `application_type`" ("native" for desktop, mobile, CLI, localhost web apps).
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration
  - Confidence: High (98)
- Finding 4.5: Scopes and WWW-Authenticate challenge.
  - Evidence quote: "MCP servers **SHOULD** include a `scope` parameter in the `WWW-Authenticate` header"; example `WWW-Authenticate: Bearer resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", scope="files:read"`; insufficient scope: "`HTTP 403 Forbidden` ... `error="insufficient_scope"` ... `scope="required_scope1 required_scope2"` ... `resource_metadata`"; "servers **SHOULD** include all scopes required for the current operation in a single challenge"; "Servers **MUST** account for scope hierarchies". Refresh tokens: servers "**SHOULD NOT** include `offline_access` in `WWW-Authenticate` scope or Protected Resource Metadata `scopes_supported`".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
  - Confidence: High (98)
- Finding 4.6: Resource indicators.
  - Evidence quote: "MCP clients **MUST** implement Resource Indicators for OAuth 2.0 as defined in RFC 8707 ... **MUST** be included in both authorization requests and token requests ... **MUST** use the canonical URI of the MCP server". Valid examples include `https://mcp.example.com/mcp`; invalid include `mcp.example.com` and `https://mcp.example.com#fragment`.
  - Source URL: same as 4.5
  - Confidence: High (98)
- Finding 4.7: RFC 9207 `iss` validation (new in 2026-07-28).
  - Evidence quote: "Before redirecting the user-agent, the client **MUST** record the `issuer` value"; "MCP authorization servers **SHOULD** include the `iss` parameter"; if `authorization_response_iss_parameter_supported` is `true` and `iss` is absent: "Reject the response"; "A future revision of this specification is expected to upgrade authorization server inclusion of `iss` from **SHOULD** to **MUST**."
  - Source URL: same as 4.5
  - Confidence: High (98)
- Finding 4.8: Enterprise-Managed Authorization (EMA) is a stable official extension using ID-JAG (Cross App Access).
  - Evidence quote: "The Enterprise-Managed Authorization extension is now stable." (blog, 2026-06-18); identifier `io.modelcontextprotocol/enterprise-managed-authorization`; "**Status**: Stable" in ext-auth `specification/stable/enterprise-managed-authorization.mdx`; "This profile is an application of the 'Identity Assertion JWT Authorization Grant' draft-ietf-oauth-identity-assertion-authz-grant"; "Okta is the first supported identity provider ... using Okta's Cross App Access (XAA)". Discovery: `urn:ietf:params:oauth:grant-profile:id-jag` in `authorization_grant_profiles_supported`.
  - Source URL: https://blog.modelcontextprotocol.io/posts/enterprise-managed-auth/ ; https://modelcontextprotocol.io/extensions/auth/enterprise-managed-authorization ; https://github.com/modelcontextprotocol/ext-auth/blob/main/specification/stable/enterprise-managed-authorization.mdx
  - Source category: Authoritative
  - Source date: 2026-06-18 (blog), ext-auth PR #29 promoted to stable (commit 2026-06-17)
  - Confidence: High (95)
- Finding 4.9: A second auth extension exists: OAuth Client Credentials.
  - Evidence quote: "OAuth Client Credentials | `io.modelcontextprotocol/oauth-client-credentials` | Machine-to-machine auth without interactive user login".
  - Source URL: https://modelcontextprotocol.io/extensions/client-matrix and https://modelcontextprotocol.io/extensions/auth/overview
  - Confidence: High (90)
- Finding 4.10: Roadmap for auth: DPoP and Workload Identity Federation are next, not yet in the spec.
  - Evidence quote: "The work here covers finalizing Demonstrating Proof of Possession (DPoP) and driving its adoption, and defining an opinionated path for agent identity and delegation through Workload Identity Federation, the ID-JAG grant behind Enterprise-Managed Authorization, and standard token exchange."
  - Source URL: https://blog.modelcontextprotocol.io/posts/mcp-roadmap/
  - Confidence: High (90)

### Question 5. Server primitives as specified in 2026-07-28

- Finding 5.1: Tools.
  - Evidence quote: Tool fields: `name`, `title`, `description`, `icons`, `inputSchema` ("**MUST** be a valid JSON Schema object (not `null`)"; no-param recommended form `{ "type": "object", "additionalProperties": false }`), `outputSchema`, `annotations`. schema.ts: `inputSchema: { $schema?: string; type: "object"; [key: string]: unknown }`, `outputSchema?: { $schema?: string; [key: string]: unknown }`, `annotations?: ToolAnnotations`, "Display name precedence order is: `title`, `annotations.title`, then `name`." `ToolAnnotations`: `title?`, `readOnlyHint?` (Default false), `destructiveHint?` (Default true), `idempotentHint?` (Default false), `openWorldHint?` (Default true). "clients **MUST** consider tool annotations to be untrusted unless they come from trusted servers." Tool names "**SHOULD** be between 1 and 128 characters", allowed chars "A-Z, a-z, digits, underscore, hyphen, and dot". `structuredContent` "can be any JSON value ... that conforms to the tool's `outputSchema` if one is defined. For backwards compatibility, a tool that returns structured content SHOULD also return the serialized JSON in a TextContent block." Content types: text, image, audio, `resource_link`, embedded `resource`. Errors: protocol errors as JSON-RPC (`-32602` unknown tool), execution errors as `isError: true`. Servers **MUST** "Validate all tool inputs, Implement proper access controls, Rate limit tool invocations, Sanitize tool outputs". Tools "**MUST NOT** vary per-connection" but "**MAY** vary by the authorization presented on the request".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/tools and schema.ts lines 1912 to 2015
  - Source category: Authoritative
  - Source date: 2026-07-28
  - Confidence: High (98)
- Finding 5.2: Stateful tools guidance (non-normative) replaces sessions.
  - Evidence quote: "Servers that need to maintain state across calls, a shopping cart, an open browser context, a database transaction, should do so by returning an explicit handle from a creation tool and accepting that handle as an argument on subsequent calls." Handle advice: authorization on every call, opacity, lifetime stated in the tool description, expiry errors as tool execution errors.
  - Source URL: same as 5.1
  - Confidence: High (95)
- Finding 5.3: Resources and templates.
  - Evidence quote: Capability `resources: { listChanged, subscribe }`; `subscribe` means "resource-specific update notifications for resources requested through subscriptions/listen using the resourceSubscriptions filter". Methods: `resources/list`, `resources/read`, `resources/templates/list` (RFC 6570 templates, completion via `completion/complete`). Resource fields: `uri`, `name`, `title`, `description`, `icons`, `mimeType`, `size`. Annotations: `audience`, `priority`, `lastModified`. Not found: "servers **MUST** return a JSON-RPC error with code `-32602`" and "clients **SHOULD** also accept `-32002`". "Servers **MUST** sanitize file paths to prevent directory traversal attacks when serving `file://` resources".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/resources
  - Confidence: High (98)
- Finding 5.4: Prompts.
  - Evidence quote: Capability `prompts: { listChanged }` "in their `DiscoverResult`"; methods `prompts/list`, `prompts/get`; fields `name`, `title`, `description`, `icons`, `arguments`; message content types text, image, audio, resource_link, embedded resource; errors `-32602` invalid name or missing args, `-32603` internal; `prompts/get` may return `InputRequiredResult`.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/prompts
  - Confidence: High (98)
- Finding 5.5: Logging is deprecated and now per-request.
  - Evidence quote: "**Deprecated**: The Logging feature is deprecated as of protocol version `2026-07-28` (SEP-2577)"; "The server **MUST NOT** emit `notifications/message` for a request that does not include this field [`io.modelcontextprotocol/logLevel`]"; "`notifications/message` is request-scoped: the server **MUST NOT** deliver it on a `subscriptions/listen` stream". Migration: stderr for stdio, OpenTelemetry.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/logging
  - Confidence: High (98)
- Finding 5.6: Completion.
  - Evidence quote: Capability `completions: {}`; method `completion/complete` with `ref/prompt` or `ref/resource`, `argument`, and `context.arguments`; results "Maximum 100 items per response", `total`, `hasMore`; "Method not found: `-32601` (Capability not supported)".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/completion
  - Confidence: High (98)
- Finding 5.7: Pagination.
  - Evidence quote: "opaque cursor-based approach"; supported by `resources/list`, `resources/templates/list`, `prompts/list`, `tools/list`; "an empty string is a valid cursor and thus **MUST NOT** be treated as the end of results"; invalid cursor `-32602`.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/pagination
  - Confidence: High (98)
- Finding 5.8: Caching (new utility).
  - Evidence quote: "Servers MUST include caching hints on results with `resultType: "complete"` returned by the following operations: `server/discover`, `tools/list`, `prompts/list`, `resources/list`, `resources/templates/list`, `resources/read`"; "Servers **MUST** provide a `ttlMs` value that is `>= 0`"; `cacheScope` `"public"` or `"private"`; MRTR retries "**MUST NOT** be cached"; "Servers **MUST** apply the same `cacheScope` to all response pages"; "MUST NOT rely on `cacheScope` alone to prevent unauthorized access".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/caching
  - Confidence: High (98)
- Finding 5.9: Progress.
  - Evidence quote: "Progress tokens **MUST** be a string or integer value ... **MUST** be unique across all active requests"; `notifications/progress` with `progressToken`, `progress`, optional `total` and `message`; "The `progress` value **MUST** increase with each notification"; "Progress notifications **MUST** stop after completion".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/progress
  - Confidence: High (98)
- Finding 5.10: Cancellation (server side is limited to listen streams).
  - Evidence quote: "A server **MUST** send `notifications/cancelled` referencing a `subscriptions/listen` request ID when it tears down that subscription stream ... Servers **MUST NOT** send `notifications/cancelled` for any other purpose."
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation
  - Confidence: High (98)
- Finding 5.11: Subscriptions (new primitive, replaces `resources/subscribe` and the GET stream).
  - Evidence quote: `subscriptions/listen` with `notifications` filter fields `toolsListChanged`, `promptsListChanged`, `resourcesListChanged`, `resourceSubscriptions`; "The server **MUST** send `notifications/subscriptions/acknowledged` as the first message carrying the subscription's ID"; "The server **MUST NOT** send notification types the client has not explicitly requested."; graceful close is a `resultType: "complete"` response; "On **stdio**, if the connection is terminated and then re-established, the client **MUST** re-send `subscriptions/listen`".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/subscriptions
  - Confidence: High (98)
- Finding 5.12: Client features (sampling, elicitation, roots) now flow through MRTR; sampling and roots are deprecated.
  - Evidence quote: "Servers **MUST** send server-to-client requests (such as `roots/list`, `sampling/createMessage`, or `elicitation/create`) using the MRTR pattern. The previous pattern of server-initiated requests is no longer supported. This is a breaking change." MRTR allowed only on `prompts/get`, `resources/read`, `tools/call`. `requestState` rules: "servers **MUST** treat `requestState` as an attacker-controlled input. If `requestState` influences authorization, resource access, or business logic, servers **MUST** protect its integrity (e.g. HMAC or AEAD)"; replay guidance: bind principal, TTL, request identifier. "Servers **MUST NOT** send an `inputRequests` that the client has not declared support for". Elicitation capability now lives in `_meta.io.modelcontextprotocol/clientCapabilities.elicitation` with `form` and `url`; "Servers **MUST NOT** use form mode elicitation to request sensitive information such as passwords, API keys, access tokens, or payment credentials". Sampling and Roots pages both open with "**Deprecated**: ... as of protocol version `2026-07-28` (SEP-2577)".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr ; .../client/elicitation ; .../client/sampling ; .../client/roots
  - Confidence: High (98)
- Finding 5.13: Primitives added or renamed in 2026-07-28: `server/discover`, `subscriptions/listen`, `notifications/subscriptions/acknowledged`, the caching utility (`CacheableResult`), `InputRequiredResult`, and `resultType`. Full method set in the schema: `completion/complete`, `elicitation/create`, `notifications/cancelled`, `notifications/message`, `notifications/progress`, `notifications/prompts/list_changed`, `notifications/resources/list_changed`, `notifications/resources/updated`, `notifications/subscriptions/acknowledged`, `notifications/tools/list_changed`, `prompts/get`, `prompts/list`, `resources/list`, `resources/read`, `resources/templates/list`, `roots/list`, `sampling/createMessage`, `server/discover`, `subscriptions/listen`, `tools/call`, `tools/list`.
  - Source URL: `grep -oE 'method: "[a-zA-Z/_]+"' schema-2026-07-28.ts` on the tagged schema
  - Confidence: High (98)
- Finding 5.14: Icons and `_meta` rules.
  - Evidence quote: Icons attach to `Implementation`, `Tool`, `Prompt`, `Resource`; clients "**MUST** reject icon URIs that use unsafe schemes and redirects, such as `javascript:`, `file:`, `ftp:`, `ws:`"; `_meta` prefixes where "the second label is `modelcontextprotocol` or `mcp` is **reserved**"; reserved keys include `progressToken`, `io.modelcontextprotocol/protocolVersion`, `clientInfo`, `clientCapabilities`, `logLevel`, `subscriptionId`, `traceparent`, `tracestate`, `baggage`.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic
  - Confidence: High (98)

### Question 6. Lifecycle

- Finding 6.1: There is no handshake in 2026-07-28.
  - Evidence quote: "There is no negotiation handshake. Every request carries its protocol version, and the server accepts or rejects each request independently". Terminology: "**Modern**: ... revision `2026-07-28` and later. **Legacy**: protocol versions that establish a session with an `initialize` handshake (`2025-11-25` and earlier). **Dual-era**".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning
  - Confidence: High (98)
- Finding 6.2: Per-request protocol fields and capability negotiation.
  - Evidence quote: Required on every request: `io.modelcontextprotocol/protocolVersion` (string) and `io.modelcontextprotocol/clientCapabilities` (ClientCapabilities). Optional: `clientInfo`, `logLevel`. "A request missing any required field is malformed; the server **MUST** reject it with JSON-RPC error code `-32602` (Invalid params). On HTTP, the response status **MUST** be `400 Bad Request`." "A server **MUST NOT** rely on capabilities the client has not declared. If processing a request requires a capability the client did not include ... the server **MUST** return a `MissingRequiredClientCapabilityError` (`-32021`)". Servers "**SHOULD** include ... `io.modelcontextprotocol/serverInfo`" in every result's `_meta`.
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic#meta
  - Confidence: High (98)
- Finding 6.3: Version negotiation and `server/discover`.
  - Evidence quote: Unsupported version: "**MUST** respond with an `UnsupportedProtocolVersionError` listing the versions it does support" (example `code: -32022`, `data.supported`, `data.requested`). "Servers **MUST** implement `server/discover`. Clients **MAY** call it before sending any other requests". `DiscoverResult` carries `supportedVersions`, `capabilities`, `_meta['io.modelcontextprotocol/serverInfo']`, `instructions`, `ttlMs`, `cacheScope`. Extension negotiation uses the `extensions` map with reverse-DNS ids; "If one party supports an extension but the other does not, the supporting party **MUST** either revert to core protocol behavior or reject the request".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/server/discover and .../basic/versioning
  - Confidence: High (98)
- Finding 6.4: `ping` is removed; shutdown is transport-level.
  - Evidence quote: Changelog: "Remove `ping`". Stdio shutdown: "The client **SHOULD** initiate shutdown by: Closing the input stream ... Waiting for the server to exit ... forcibly terminating the process"; "Servers **SHOULD** exit promptly when their standard input is closed or reads return end-of-file." Unexpected termination: "the client **SHOULD** restart it. Because the protocol is stateless, any in-flight requests are simply lost and the client can retry them".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio
  - Confidence: High (98)
- Finding 6.5: Statelessness rules bind servers.
  - Evidence quote: "Servers **MUST NOT** rely on prior requests over the same connection to establish context"; "State that needs to span multiple requests ... **MUST** be referenced by an explicit identifier the client passes on each request."; "an open connection, such as a STDIO process, is not a conversation or session".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic#statelessness
  - Confidence: High (98)
- Finding 6.6: Dual-era server behavior and the legacy handshake for reference.
  - Evidence quote: "A dual-era **server** selects its behavior from how the client opens: A request carrying modern per-request `_meta` is served statelessly ... An `initialize` request selects legacy semantics"; "A server that supports only modern versions **SHOULD** name the protocol versions it supports in any error it returns to an `initialize` request". The legacy 2025-11-25 lifecycle: "The client **MUST** initiate this phase by sending an `initialize` request" and "the client **MUST** send an `initialized` notification".
  - Source URL: https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning and https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
  - Confidence: High (98)

### Question 7. Governance and ecosystem

- Finding 7.1: MCP is governed under the Linux Foundation's Agentic AI Foundation.
  - Evidence quote: "Model Context Protocol has been established as **Model Context Protocol a Series of LF Projects, LLC**." (governance page). "Anthropic is donating the Model Context Protocol (MCP) to the Agentic AI Foundation (AAIF), a directed fund under the Linux Foundation, co-founded by Anthropic, Block and OpenAI, with support from Google, Microsoft, Amazon Web Services (AWS), Cloudflare, and Bloomberg." (Anthropic, 2025-12-09). "The AAIF Governing Board will make decisions regarding strategic investments, budget allocation, member recruitment, and approval of new projects, while individual projects, such as MCP, maintain full autonomy over their technical direction" (MCP blog, 2025-12-09).
  - Source URL: https://modelcontextprotocol.io/community/governance ; https://www.anthropic.com/news/donating-the-model-context-protocol-and-establishing-of-the-agentic-ai-foundation ; https://blog.modelcontextprotocol.io/posts/2025-12-09-mcp-joins-agentic-ai-foundation/ ; https://www.linuxfoundation.org/press/linux-foundation-announces-the-formation-of-the-agentic-ai-foundation
  - Source category: Authoritative
  - Source date: 2025-12-09 (announcements); governance page read 2026-09-12
  - Confidence: High (98)
- Finding 7.2: Steering structure and current people.
  - Evidence quote: "Together, Maintainers, Core Maintainers, and Lead Maintainers form the **MCP Steering Group**." "Current Lead Maintainers: David Soria Parra, Den Delimarsky". "Current Core Maintainers: Peter Alexander, Caitie McCaffrey, Kurtis Van Gent, Clare Liguori, Paul Carleton, Nick Cooper". Emeritus includes "Justin Spahr-Summers (Co-Inventor, Lead Maintainer Emeritus)". "The Core Maintainer group meets every two weeks". Licenses: Apache-2.0 for code and spec, CC BY 4.0 for docs.
  - Source URL: https://modelcontextprotocol.io/community/governance
  - Confidence: High (95)
- Finding 7.3: AAIF executive director change (secondary, for context).
  - Evidence quote: "Mazin Gilbert, the new executive director of the new AAIF" (The New Stack, 2026-05-06).
  - Source URL: https://thenewstack.io/agentic-ai-foundation-launch/
  - Source category: Consensus
  - Source date: 2026-05-06
  - Confidence: Medium (70), single reputable secondary source.
- Finding 7.4: The MCP Registry is in preview; API v0.1 frozen; live version 1.8.1.
  - Evidence quote: "The MCP Registry is currently in preview. Breaking changes or data resets may occur before general availability." (about, quickstart, and authentication pages). Live endpoint `https://registry.modelcontextprotocol.io/v0.1/version` returned `{"version":"1.8.1","git_commit":"f52dc852...","build_time":"2026-08-06T23:41:04Z"}`. GitHub releases: v1.8.1 (2026-08-06), v1.8.0 (2026-07-13). README: "**2025-10-24 update**: The Registry API has entered an **API freeze (v0.1)**".
  - Source URL: https://modelcontextprotocol.io/registry/about ; https://registry.modelcontextprotocol.io/v0.1/version ; https://github.com/modelcontextprotocol/registry
  - Source category: Authoritative
  - Source date: read 2026-09-12
  - Confidence: High (95)
- Finding 7.5: How a server gets listed.
  - Evidence quote: Publish with the `mcp-publisher` CLI: `mcp-publisher init`, `mcp-publisher login`, `mcp-publisher publish`, plus `validate` and `status` commands; `server.json` uses `"$schema": "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json"`; names are `io.github.username/*` (GitHub auth) or reverse-DNS `com.example.*/*` (DNS or HTTP verification); "For npm packages, this requires adding an `mcpName` property to `package.json`"; "The MCP Registry only hosts metadata, not artifacts"; "Only trusted public registries are supported. Private registries and alternative mirrors are not allowed."; the registry "**does not** support private servers". Auth methods: GitHub OAuth, GitHub OIDC, DNS verification, HTTP verification. Status values: `active`, `deprecated`, `deleted`.
  - Source URL: https://modelcontextprotocol.io/registry/quickstart ; https://modelcontextprotocol.io/registry/authentication ; https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/api/official-registry-api.md
  - Confidence: High (95)
- Finding 7.6: Official SDK support matrix.
  - Evidence quote: "TypeScript | Tier 1; Python | Tier 1; C# | Tier 1; Go | Tier 1; Rust | Tier 1; Java | Tier 2; Ruby | Tier 2; Swift | Tier 3; PHP | Tier 3; Kotlin | Tier 3".
  - Source URL: https://modelcontextprotocol.io/docs/2026-07-28/sdk
  - Source category: Authoritative
  - Source date: read 2026-09-12
  - Confidence: High (95)
- Finding 7.7: Tier definitions.
  - Evidence quote: Tier 1 requires "100% pass rate" on conformance, new features "Before new spec version release", triage "Within 2 business days", P0 fixes "Within 7 days", stable release, published dependency policy and roadmap. Tier 2: 80 percent, features within 6 months. Relegation: "Tier 1 → Tier 2: Any conformance test fails" for 4 weeks. Key dates: "January 23, 2026: Conformance tests available; February 23, 2026: Official SDK tiering published".
  - Source URL: https://modelcontextprotocol.io/community/sdk-tiers
  - Confidence: High (95)
- Finding 7.8: Rust SDK status in detail.
  - Evidence quote: PR #3287 "Promote Rust SDK to Tier 1 in the SDK listing", merged 2026-08-21T12:30:14Z, body: "server conformance 67/67, client conformance 50/50, 12/12 labels, stable rmcp 3.0.1, same-day spec tracking, and VERSIONING / DEPENDENCY_POLICY / ROADMAP published." crates.io: `rmcp` max_stable_version 3.3.0, updated 2026-09-10T15:32:23Z, 25,913,092 downloads; 3.0.0 created 2026-07-28T22:52:36Z. README: "This SDK implements the stable MCP **`2026-07-28`** specification while remaining fully compatible with the **`2025-11-25`** release and earlier versions." Workspace: `edition = "2024"`, `rust-version = "1.88"`, `version = "3.3.0"`. ROADMAP: "Conformance is 100% across every date-versioned suite" (2025-11-25 and 2026-07-28, server 30/30 each, client 100%). Source constants: `V_2026_07_28`, `LATEST = V_2025_11_25`, `STANDARD_HEADERS = V_2026_07_28`, `KNOWN_VERSIONS` includes all five dated versions. Migration guide: "RMCP 3.0.0 is stable ... shipped on 2026-07-28 via release PR #1077"; API shape: `ServerHandler::call_tool` returns `CallToolResponse`, `ServerHandler::discover` has a default, `StreamableHttpServerConfig::with_legacy_session_mode`, `ClientLifecycleMode::Discover | Auto | Initialize`, `RequestStateCodec` (feature `request-state`) for HMAC-sealed `requestState`, `Peer::listen(SubscriptionFilter)`. Known limitation in PR #1020: "`notifications/tasks` push via `subscriptions/listen` is not yet wired".
  - Source URL: https://api.github.com/repos/modelcontextprotocol/modelcontextprotocol/pulls/3287 ; https://crates.io/api/v1/crates/rmcp ; https://github.com/modelcontextprotocol/rust-sdk (README, ROADMAP.md, crates/rmcp/src/model.rs, crates/rmcp/CHANGELOG.md) ; https://github.com/modelcontextprotocol/rust-sdk/discussions/969 ; https://github.com/modelcontextprotocol/rust-sdk/pull/1020
  - Source category: Authoritative
  - Source date: 2026-08-21 (PR), 2026-09-10 (crate), repo pushed 2026-09-12T01:29:25Z
  - Confidence: High (95)
- Finding 7.9: Rust SDK security advisory (historical, patched).
  - Evidence quote: RUSTSEC-2026-0189, `date = "2026-04-29"`, aliases `CVE-2026-42559`, `GHSA-89vp-x53w-74fx`: "Prior to version 1.4.0, the `rmcp` crate's Streamable HTTP server transport did not validate the incoming `Host` header." `patched = [">= 1.4.0"]`.
  - Source URL: https://raw.githubusercontent.com/rustsec/advisory-db/main/crates/rmcp/RUSTSEC-2026-0189.md
  - Source category: Standards (advisory database)
  - Source date: 2026-04-29 (older than the 30-day advisory window; reported as history, patched long before rmcp 3.x)
  - Confidence: High (95)
- Finding 7.10: Conformance tooling.
  - Evidence quote: Repo `modelcontextprotocol/conformance`: "`--spec-version` - Filter scenarios by spec version (e.g., `2025-11-25`, `2026-07-28`...)"; "`npx @modelcontextprotocol/conformance server --url http://localhost:3000/mcp --requirements 2026-07-28`"; `tier-check` subcommand "evaluates an MCP SDK repository against SEP-1730". npm: `latest` 0.1.16 (2026-03-30), `alpha` 0.2.0-alpha.11. RC blog: "a Standards Track SEP can no longer reach Final status until a matching scenario lands in the conformance suite (SEP-2484)". Issue #426 (2026-07-31): "The current conformance `main` ... predates the final MCP `2026-07-28` release ... and still classifies `2026-07-28` as the draft protocol version".
  - Source URL: https://github.com/modelcontextprotocol/conformance ; https://registry.npmjs.org/@modelcontextprotocol/conformance ; https://github.com/modelcontextprotocol/conformance/issues/426
  - Source category: Authoritative
  - Source date: read 2026-09-12; issue 2026-07-31
  - Confidence: High (90); see Limitations on the alpha tag.
- Finding 7.11: Inspector.
  - Evidence quote: "It ships as a single package, `@modelcontextprotocol/inspector`, providing **three clients behind one binary**" (Web, `--cli`, `--tui`); "the same protocol-era negotiation (legacy vs. modern 2026-07-28)"; "The Inspector requires **Node 22.19.0 or newer**". npm dist-tags: `latest` 2.6.0 (2026-09-09T16:24:51Z), `v1-latest` 1.0.2 (2026-08-24), `next` 2.0.0-rc.3. GitHub 2.0.0 release: "The v2 Inspector is now the default release." Inspector V2 WG charter adopted 2026-04-11.
  - Source URL: https://modelcontextprotocol.io/docs/2026-07-28/tools/inspector ; https://registry.npmjs.org/@modelcontextprotocol/inspector ; https://github.com/modelcontextprotocol/inspector/releases/tag/2.0.0 ; https://modelcontextprotocol.io/community/working-groups/inspector-v2
  - Confidence: High (95)
- Finding 7.12: Official extensions inventory.
  - Evidence quote: Extensions overview lists ext-auth (OAuth Client Credentials, Enterprise-Managed Authorization), ext-apps (MCP Apps, `io.modelcontextprotocol/ui`, spec 2026-01-26 Stable, blog 2026-01-26 "the first official MCP extension"), and MCP Tasks (`io.modelcontextprotocol/tasks`, ext-tasks schema `2026-07-28` Stable). Experimental: Skills (SEP-2640 Draft, `io.modelcontextprotocol/skills`, repo experimental-ext-skills) and Server Card (SEP-2127, `io.modelcontextprotocol/server-card`, repo experimental-ext-server-card). "SDKs MAY implement extensions. Where implemented, extensions MUST be disabled by default and require explicit opt-in." (SEP-2133).
  - Source URL: https://modelcontextprotocol.io/extensions/overview ; https://modelcontextprotocol.io/extensions/client-matrix ; https://github.com/modelcontextprotocol/ext-apps ; https://raw.githubusercontent.com/modelcontextprotocol/ext-tasks/main/README.md ; https://modelcontextprotocol.io/community/working-groups/skills-over-mcp ; https://modelcontextprotocol.io/community/working-groups/server-card
  - Confidence: High (92)
- Finding 7.13: Tasks extension mechanics (relevant to long-running DBA operations).
  - Evidence quote: "the server returns a `CreateTaskResult` (identified by `resultType: "task"`) containing a `taskId`, initial status, TTL, and suggested polling interval. The task is durably created before the response is sent."; statuses `working`, `input_required`, `completed`, `failed`, `cancelled`; "A server MUST NOT return `CreateTaskResult` to a client that did not include the extension capability on its request"; over HTTP "the client MUST set the `Mcp-Name` header to the value of `params.taskId`"; "The `notifications/cancelled` notification MUST NOT be used for task cancellation"; errors `-32602` unknown task, `-32021` missing capability.
  - Source URL: https://modelcontextprotocol.io/extensions/tasks/overview ; https://modelcontextprotocol.io/seps/2663-tasks-extension ; https://tasks.extensions.modelcontextprotocol.io/specification/draft/tasks
  - Confidence: High (95)
- Finding 7.14: Feature lifecycle policy details.
  - Evidence quote: States Active, Deprecated, Removed; "the number of months, at least twelve, that the feature must remain Deprecated"; "The window is measured from the release of the specification revision in which the feature is first marked Deprecated"; expedited removal "must still provide at least ninety days"; Tier 1 SDKs "Must mark the corresponding API surface deprecated using the language's native mechanism".
  - Source URL: https://modelcontextprotocol.io/community/feature-lifecycle
  - Confidence: High (98)

### Question 8. What a server author must not do (security best practices)

All quotes from https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices (Authoritative, 2026-07-28 docs tree) unless noted. Confidence: High (98).

- Token passthrough: "MCP servers **MUST NOT** accept any tokens that were not explicitly issued for the MCP server." Authorization spec adds: "MCP servers **MUST NOT** accept or transit any other tokens." and "The MCP server **MUST NOT** pass through the token it received from the MCP client."
- State handles: "MCP servers that implement authorization **MUST** verify all inbound requests. MCP servers **MUST NOT** treat possession of a state handle as authentication." Handles "**SHOULD** use secure, non-deterministic handles" and be bound server-side as `<user_id>:<handle>`.
- Confused deputy (proxy servers): "MCP proxy servers **MUST** implement per-client consent"; "The consent cookie or session containing the `state` value **MUST NOT** be set until **after** the user has approved the consent screen".
- Local servers: "Use the `stdio` transport to limit access to just the MCP client"; if HTTP, "Require an authorization token" or "Use unix domain sockets". Transport spec: validate `Origin` (403 on mismatch), bind to 127.0.0.1.
- URL-mode elicitation (server side): "**MUST NOT** include sensitive information about the end-user ... in the URL"; "**MUST NOT** provide a URL which is pre-authenticated to access a protected resource"; "The MCP Server **MUST** verify the identity of the user who opens the URL before accepting information." Form mode: "**MUST NOT** request sensitive information (passwords, API keys, etc.) via form mode".
- MRTR `requestState`: "servers **MUST** treat `requestState` as an attacker-controlled input" and "**MUST** reject state that fails verification" when it influences authorization or logic.
- Headers: servers "**SHOULD NOT** mark sensitive parameters (passwords, API keys, tokens, PII) with `x-mcp-header`". Servers "**MUST** reject requests where the values specified in the headers do not match the corresponding values in the request body."
- Schemas: "Implementations **MUST NOT** automatically dereference `$ref` values that resolve to a network URI." Bound composition-keyword validation cost.
- Logging (if used at all, and it is deprecated): "Log messages **MUST NOT** contain: Credentials or secrets, Personal identifying information, Internal system details that could aid attacks".
- Tools: servers **MUST** "Validate all tool inputs, Implement proper access controls, Rate limit tool invocations, Sanitize tool outputs". Resources: "**MUST** sanitize file paths to prevent directory traversal".
- Caching: "MUST NOT rely on `cacheScope` alone to prevent unauthorized access to primitives."
- Scopes: "Common Mistakes: Publishing all possible scopes in `scopes_supported`; Using wildcard or omnibus scopes (`*`, `all`, `full-access`) ... Treating claimed scopes in token as sufficient without server-side authorization logic".
- Sessions: do not depend on `Mcp-Session-Id` or any prior request for context ("Servers **MUST NOT** rely on prior requests over the same connection to establish context").
- Error codes: "Implementations **MUST NOT** emit any code from this sub-range [-32020 to -32099] that is not defined by this specification"; do not emit `-32002` or `-32042`.

## 3. Conflicts found and resolution

- Rust SDK tier. The 2026-07-28 blog says "Beyond the Tier 1 set, the Rust SDK supports the new spec in beta." The SDK page lists Rust as Tier 1. Resolution: both were true at their dates. rmcp 3.0.0 stable was published 2026-07-28T22:52:36Z, hours after the 09:00 UTC blog post, and PR #3287 promoted Rust to Tier 1 on 2026-08-21. The current state is Tier 1. Authority: GitHub PR API and crates.io API over the blog.
- RC name "2026-06-30". SEP-2663 text and a Keycloak issue reference a "2026-06-30" specification. Resolution: the RC was first targeted for 2026-06-30 and renamed to 2026-07-28 (PR #2750 title rename; Keycloak note "The release date was changed from 06-30 to 07-28"). No revision named 2026-06-30 exists in the schema directory or tags.
- "Release candidate" wording after GA. The Google Developers Blog (2026-08-05) and the AAIF migration post describe 2026-07-28 as a release candidate. Resolution: the GitHub release (`prerelease: false`, "stable release") and the official blog on 2026-07-28 settle it as final. The AAIF post carries its own note that it covers the RC.
- Rust `ProtocolVersion::LATEST`. `model.rs` sets `LATEST = V_2025_11_25` while the README claims `2026-07-28` support. Resolution: `LATEST` is the SDK's default client negotiation target; `V_2026_07_28` is a supported constant, `STANDARD_HEADERS = V_2026_07_28`, servers serve 2026-07-28 statelessly by default, and clients opt into the modern lifecycle through `ClientLifecycleMode::Discover` or `Auto`. Not a contradiction, but a default a builder must set deliberately. Confidence on this reading: Medium (80), since it rests on code reading rather than an explicit doc sentence.
- Registry GA. Some third-party pages (Portkey, Latenode) imply GA. Resolution: every official registry page still carries "currently in preview", and the API remains v0.1. Preview stands.
- Tool annotations. The 2026-07-28 Tools page prose no longer enumerates the hint names. Resolution: `ToolAnnotations` in the tagged `schema.ts` still defines all four hints plus `title`; the MCP blog (2026-03-16) and the changelog list no change. Unchanged.
- HTTP+SSE earliest removal. The blog says "a year-long offramp"; the deprecated registry says earliest removal is "Three months after SEP-2596 reaches Final", and the page states it "is a derived view kept consistent with the per-feature deprecation notices and changelog entries, which are the normative records." Resolution: follow the registry (eligible for removal in the next revision), and treat the blog wording as informal. Confidence on the exact date: Medium (70).
- Conformance npm tags. `latest` is 0.1.16 (2026-03-30) while Tier 1 verification used `0.2.0-alpha.11`, and issue #426 (2026-07-31) reported tier-check treating 2026-07-28 as draft. Resolution: the alpha line is the one that scores 2026-07-28; report as a limitation rather than a contradiction.

## 4. Open questions (below 50 percent)

- When did Inspector 2.0.0 become the npm `latest` default? MCPJam says July 28, 2026; the GitHub release page confirms v2 is the default but I could not capture its publish date (GitHub API rate-limited at the end of the session). Searched: Serper, Exa, Brave on Inspector 2.x releases.
- Is there any planned GA date for the MCP Registry? Searched: Exa, Serper, WebSearch on registry status. No official date found.
- Has SEP-2127 (Server Card) moved from `experimental-ext-server-card` to an official `ext-server-card` repo? The SEP file on a branch shows Status Final, but the WG page still points at the experimental repo. Searched: Exa on Server Card WG.
- Will the next revision remove `ping` compatibility paths from SDKs or restore anything? Draft is empty as of 2026-09-08 main; nothing to report.

## 5. Methodology

- Session timestamp: September 12 2026, 01:42:45 PM (+0600), from `TZ='Asia/Dhaka' date`.
- Searches run per tool:
  - Exa (`mcp__exa__web_search_exa`): 20
  - Serper (`mcp__serper__google_search`): 20
  - Exa versus Serper gap: 0
  - Tavily (`mcp__tavily__tavily_search`): 5 successful (one additional attempt returned HTTP 429 and was retried with a varied query per the skip rule; the tool recovered on attempt two and was never marked dead)
  - Brave (`mcp__brave-search__brave_web_search`): 5
  - Built-in WebSearch: 5
  - Serper specialized indexes: none used, so no tilt.
- Primary sources fetched and read in full with the local reader (`mcp__read-website__read_website`): 44 pages, all returned full body text. They include the versioning docs page, the 2026-07-28 spec index, changelog, deprecated registry, basic overview, versioning and compatibility, server/discover, transports overview, stdio, Streamable HTTP, MRTR, subscriptions, cancellation, progress, tools, resources, prompts, caching, completion, logging, pagination, authorization overview, authorization server discovery, client registration, authorization security considerations, elicitation, sampling, roots, security best practices, draft changelog, blog index, 2026-07-28 release post, RC post, roadmap post, tasks overview, SDK page, SDK tiers, governance, feature lifecycle, registry about, Inspector docs, AAIF migration post, Claude blog post, and the GitHub release page.
- Registry and repository API reads via curl (local, free): GitHub API (spec repo tags, releases, schema directory, draft directory, commits, PR #3287, rust-sdk repo and releases, registry releases), raw GitHub (schema.ts for 2026-07-28 and draft, rust-sdk README, model.rs, CHANGELOG.md, Cargo.toml, ROADMAP.md, ext-tasks README, RustSec advisory), crates.io API (rmcp crate and versions), npm registry API (inspector, conformance), and the live registry `/v0.1/version` and `/v0.1/health` endpoints. About 22 reads.
- Paid fetchers used: 0 (no `tavily_extract`, `web_fetch_exa`, or `webpage_scrape` calls were needed; no local read came back thin).
- Sources evaluated versus selected: roughly 300 search result entries across the five engines (about 90 unique domains); 44 primary pages plus about 22 API and raw-file reads were read in full and cited; about 12 secondary pages (Go SDK docs, TrueFoundry, Descope, WorkOS, MCPJam, ChatForest, digitalapplied, nomadLab, TensorFoundry, Appwrite, Skycloak, Portkey) were used only as corroboration or discarded.
- False positives discarded: nomadLab post dated 2026-07-21 describing a 2026-07-28 ship date (date inconsistency); Google Developers Blog and AAIF posts calling the final spec a "release candidate" (used only for context); an older Exa-indexed snapshot of the ext-tasks README marked "Experimental" (superseded by the current raw README marked official and Stable); the webfuse cheat sheet listing Rust as Tier 2 (stale); Portkey and Latenode registry GA implications (contradicted by official pages); Medium and Zhihu posts (leads only); `mcp-staging.mintlify.app` and `modelcontextprotocol.org` mirrors (duplicates of the official site).
- Depth honored: exhaustive for the eight numbered questions; every current-state claim rests on the spec page, the tagged schema, or a registry or repository API.
- Tools marked dead: none.

## 6. Limitations

- GitHub's unauthenticated API returned 403 (rate limit) at the very end of the session, so the 2026-07-28 release body was read from the HTML page instead of the API; the earlier API reads (tags, releases, PR #3287) completed before the limit.
- The exact earliest-removal date for HTTP+SSE depends on when SEP-2596 reached Final; the registry says "Three months after SEP-2596 reaches Final" without a calendar date.
- The conformance suite's `latest` npm tag (0.1.16) predates the final spec; scoring against 2026-07-28 uses the `0.2.0-alpha.*` line. Anyone running `tier-check` should pin the alpha and pass `--requirements 2026-07-28`.
- The Rust SDK's default client version target is `2025-11-25` (`ProtocolVersion::LATEST`); a client built on rmcp must pick `ClientLifecycleMode::Discover` or `Auto` with `V_2026_07_28` to speak the modern era. Servers built on rmcp serve 2026-07-28 statelessly by default. This reading comes from source code and the README rather than a single normative sentence.
- The Tasks extension's `notifications/tasks` push over `subscriptions/listen` was noted as "not yet wired" in rmcp PR #1020; I did not verify whether rmcp 3.3.0 closed that gap.
- The Extension Support Matrix page renders client support as icons that the local reader could not capture, so per-client extension support was not recorded.
- No fetched page contained instructions aimed at this agent.
- Items needing the Principal Architect's verification: none beyond the Medium-confidence items above (AAIF executive director, HTTP+SSE removal date, and the rmcp default-version reading).
