# ADR-0014: Remote mode is an OAuth 2.1 resource server with an external identity provider, plus a static bearer mode

**Status**: Accepted (by authorizing phase 0 on this pack, 2026-09-12)
**Date**: 2026-09-12
**Authors**: Md. Sazzad Hossain Sharkar
**Decided by**: Md. Sazzad Hossain Sharkar, Principal Architect

## Context

Remote mode is the brief's secondary target. The `2026-07-28` authorization specification makes the server an OAuth 2.1 resource server that must publish Protected Resource Metadata (RFC 9728), validate token audience (RFC 8707), never pass tokens through, and answer 401 with a `WWW-Authenticate` challenge; clients prefer Client ID Metadata Documents and use PKCE S256. claude.ai and Claude Desktop connect from Anthropic's cloud (`160.79.104.0/21`) over HTTPS on 443 and need CIMD (or dynamic registration as fallback); Claude Code runs its own loopback OAuth; Claude Code, Cursor, VS Code, Codex, Gemini CLI, Zed, and Windsurf also accept a static `Authorization` header. rmcp provides no server-side validation. Auth0, WorkOS AuthKit, and Clerk meet the requirements; Keycloak lacks RFC 8707; Ory Hydra lacks CIMD. How does OwnPG authenticate remote clients?

## Decision Drivers

- Spec compliance so claude.ai and other hosted clients can connect.
- No identity provider inside OwnPG (no user store, no password handling).
- A private-deployment path that needs no identity provider at all.
- Scopes map onto access modes so the token, not a flag, decides what a client may do.

## Options Considered

### Option A: Resource server with external IdP, plus `auth = "bearer"` and `auth = "none"` (loopback only)

`serve --http --auth oauth`: serve `/.well-known/oauth-protected-resource` naming the configured authorization server(s); validate bearer JWTs through JWKS (or RFC 7662 introspection when the IdP requires it), where the JWKS client applies a 10 second connect and 30 second overall timeout, a 1 MiB response cap, honors `HTTPS_PROXY`, sends `ownpg/<version>` as its user agent, caches keys for the response's cache window, refreshes on an unknown key id at most once per minute, and fails closed with 401 when the endpoint is unreachable; check `aud` against the server's canonical URL, `iss`, `exp`, and scopes; answer 401 with `resource_metadata` and `scope`, 403 with `insufficient_scope`; map scopes onto tool groups (`ownpg:read` for the default set and `monitoring`, `ownpg:write` for `write` and `transactions`, `ownpg:ddl`, `ownpg:roles`, `ownpg:maintenance`, `ownpg:host`); document the WorkOS AuthKit recipe first with CIMD enabled (decided 2026-09-12), with Auth0 and Clerk noted as alternatives that also meet the requirements. `serve --http --auth bearer`: one or more static tokens from a 0600 file or the environment, constant-time comparison, each token bound to a mode. `serve --http --auth none`: allowed only on loopback addresses. All modes: TLS terminated by a reverse proxy (Caddy or nginx) or Cloudflare Tunnel, `Origin` and `Host` validation, body cap, a rate limit of 60 calls per minute per token and per client address answered with 429 and `Retry-After`, and a health endpoint outside `/mcp` with separate liveness and readiness answers.

- Pros: claude.ai custom connectors work; private deployments need only a token.
- Pros: rmcp's Tower service composes with axum middleware; the auth examples in the SDK show the shape.
- Cons: the JWT validation, metadata document, and challenge are OwnPG's code (a few hundred lines with `jsonwebtoken` and `reqwest` for JWKS).

### Option B: Built-in authorization server (OwnPG issues its own tokens)

- Pros: no external dependency.
- Cons: OwnPG would own user accounts, consent screens, token storage, and the whole OAuth attack surface; the spec's confused-deputy section applies in full.

### Option C: Bearer tokens only

- Pros: simplest.
- Cons: claude.ai and Claude Desktop accept static headers only through a beta org-admin feature; no per-user identity in the audit.

### Option D: Do nothing (no auth on HTTP)

- Cons: CVE-2026-61742 and CVE-2026-59971 are what that looks like; the spec says servers should authenticate every connection.

## Decision

We will make remote mode an OAuth 2.1 resource server against an external identity provider that supports PKCE and CIMD, add a static bearer mode for private deployments, allow no authentication only on loopback, and bind scopes to tool groups.

Option A won because it is the only design that both satisfies the specification and keeps OwnPG out of the identity business.

## Consequences

- Positive: a scope-limited token cannot reach write or DDL tools, whatever the process flags say.
- Positive: audit records carry the token subject.
- Negative: users who choose Keycloak must wait for its RFC 8707 support; the README says so.
- Negative: JWKS fetching adds an outbound HTTPS dependency in remote mode (the only outbound call OwnPG ever makes).
- Neutral: Enterprise-Managed Authorization and Client Credentials extensions are out of scope for 1.0.

## Reversibility

Expensive to reverse for the scope model once tokens are issued; cheap for the choice of identity provider.

## Sources

- https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization and its discovery, client-registration, and security-considerations pages
- https://claude.com/docs/connectors/building/authentication and https://platform.claude.com/docs/en/api/ip-addresses (retrieved 2026-09-12)
- https://www.keycloak.org/securing-apps/mcp-authz-server and https://github.com/ory/hydra/issues/4061
- `research/02-rust-mcp-sdk-rmcp.md`, question 4 (rmcp auth is client-side); `research/06-mcp-tool-design-security-clients-distribution.md`, findings 24 to 27
- `research/03-existing-postgresql-mcp-servers.md`, question 5 (CVE-2026-61742, CVE-2026-59971)
