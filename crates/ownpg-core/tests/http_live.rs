#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing
)]

mod support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use ownpg_core::audit::{Sink, Transport};
use ownpg_core::config::{AuthMode, FlagLayer, HttpFlags, Mode, ToolGroup};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::engine::Engine;
use ownpg_core::server::{Principal, Server, http};
use ring::signature::KeyPair;
use serde_json::{Value, json};

const PROTOCOL: &str = "2026-07-28";
const TOKEN_READ: &str = "reader-token-0123456789abcdef";
const TOKEN_WRITE: &str = "writer-token-0123456789abcdef";
const TOKEN_WRITE_ONLY: &str = "write-only-token-0123456789abcdef";

struct Remote {
    base: String,
    client: reqwest::Client,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<ownpg_core::Result<ownpg_core::ExitClass>>,
}

async fn remote(
    scratch: &support::Scratch,
    auth: Option<AuthMode>,
    extra: &[(&str, &str)],
) -> Remote {
    let settings = Arc::new(scratch.settings_with(
        FlagLayer {
            schema: Some("app".to_owned()),
            mode: Some(Mode::ReadWrite),
            tools: Some(vec![ToolGroup::Write]),
            http: HttpFlags {
                enabled: true,
                bind: Some("127.0.0.1:0".to_owned()),
                auth,
            },
            ..FlagLayer::default()
        },
        extra,
    ));
    let engine = Engine::start_pooled(Arc::clone(&settings), Hints::default())
        .await
        .expect("the pooled engine starts");
    assert_eq!(engine.pool_size(), Some(4));
    let server = Arc::new(
        Server::new(
            Arc::new(engine),
            Arc::new(Sink::disabled()),
            Transport::Http,
            Principal::local(None),
        )
        .await
        .expect("the server builds"),
    );
    let environment_tokens = extra
        .iter()
        .find(|(name, _)| *name == "OWNPG_BEARER_TOKENS")
        .map(|(_, value)| (*value).to_owned());
    let gate = Arc::new(
        http::gatekeeper(
            Arc::clone(&server),
            &settings,
            environment_tokens.as_deref(),
            Principal::local(None),
        )
        .expect("the gatekeeper builds"),
    );
    let router = http::router(Arc::clone(&gate), &settings);
    let listening = http::Listening::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let base = format!("http://{}", listening.local_addr);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(http::serve(
        listening,
        router,
        gate,
        async move {
            let _ = stopped.await;
        },
        Duration::from_secs(5),
    ));
    let _ = rustls::crypto::ring::default_provider().install_default();
    Remote {
        base,
        client: reqwest::Client::builder().build().unwrap(),
        stop: Some(stop),
        task,
    }
}

impl Remote {
    fn call_body(&self, tool: &str, arguments: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                    "io.modelcontextprotocol/clientCapabilities": {}
                },
                "name": tool,
                "arguments": arguments
            }
        })
    }

    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        let method = body["method"].as_str().unwrap_or("tools/list").to_owned();
        self.client
            .post(format!("{}/mcp", self.base))
            .header("MCP-Protocol-Version", PROTOCOL)
            .header("Mcp-Method", method)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(body.to_string())
    }

    async fn tool(
        &self,
        tool: &str,
        token: Option<&str>,
    ) -> (u16, Value, reqwest::header::HeaderMap) {
        let body = self.call_body(tool, json!({}));
        let mut request = self.post(&body).header("Mcp-Name", tool);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap();
        let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, value, headers)
    }

    async fn finish(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task
            .await
            .unwrap()
            .expect("the http server stops cleanly");
    }
}

fn result_of(value: &Value) -> &Value {
    &value["result"]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_endpoint_answers_calls_and_enforces_the_transport_rules() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app; CREATE TABLE app.t (id int)")
        .await
        .unwrap();
    let remote = remote(&scratch, None, &[]).await;

    let live = remote
        .client
        .get(format!("{}/healthz/live", remote.base))
        .send()
        .await
        .unwrap();
    assert_eq!(live.status(), 200);
    let ready = remote
        .client
        .get(format!("{}/healthz/ready", remote.base))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
    assert_eq!(ready.text().await.unwrap(), "ready");

    let (status, body, headers) = remote.tool("pg_health", None).await;
    assert_eq!(status, 200, "{body}");
    assert_ne!(result_of(&body)["isError"], true, "{body}");
    assert!(
        result_of(&body)["structuredContent"]["checks"]
            .as_array()
            .unwrap()
            .len()
            >= 7
    );
    assert_eq!(
        headers.get("x-accel-buffering").unwrap().to_str().unwrap(),
        "no"
    );

    let list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = remote.post(&list).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let listed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result_of(&listed)["ttlMs"], 60_000);
    assert_eq!(result_of(&listed)["cacheScope"], "private");

    let mismatch = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/list",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2025-11-25",
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = remote.post(&mismatch).send().await.unwrap();
    assert_eq!(response.status(), 400);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], -32020, "{error}");

    let get = remote
        .client
        .get(format!("{}/mcp", remote.base))
        .header("Accept", "text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL)
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 405);
    assert_eq!(get.headers().get("allow").unwrap(), "POST");
    let delete = remote
        .client
        .delete(format!("{}/mcp", remote.base))
        .header("MCP-Protocol-Version", PROTOCOL)
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), 405);

    let foreign_origin = remote
        .post(&remote.call_body("pg_health", json!({})))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_origin.status(), 403);
    let foreign_host = remote
        .post(&remote.call_body("pg_health", json!({})))
        .header("Host", "evil.example")
        .send()
        .await
        .unwrap();
    assert!(
        [403, 421].contains(&foreign_host.status().as_u16()),
        "{}",
        foreign_host.status()
    );

    let huge = "x".repeat(1024 * 1024 + 1);
    let oversized = remote
        .client
        .post(format!("{}/mcp", remote.base))
        .header("MCP-Protocol-Version", PROTOCOL)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(huge)
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), 413);

    let metadata = remote
        .client
        .get(format!(
            "{}/.well-known/oauth-protected-resource/mcp",
            remote.base
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata.status(), 200);
    let document: Value = metadata.json().await.unwrap();
    assert!(
        document["resource"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:")
    );
    assert!(document["resource"].as_str().unwrap().ends_with("/mcp"));
    assert_eq!(document["resource_name"], "OwnPG");
    assert_eq!(document["authorization_servers"], Value::Null);

    let mut last = 200;
    let mut retry_after = None;
    let mut throttled = Value::Null;
    for _ in 0..70 {
        let (status, body, headers) = remote.tool("pg_health", None).await;
        last = status;
        if status == 429 {
            retry_after = headers
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            throttled = body;
            break;
        }
    }
    assert_eq!(last, 429);
    assert!(retry_after.is_some_and(|seconds| seconds >= 1));
    assert_eq!(throttled["jsonrpc"], "2.0", "{throttled}");
    assert_eq!(throttled["error"]["code"], http::RATE_LIMITED_CODE);
    assert_eq!(
        throttled["error"]["data"]["retry_after_seconds"].as_u64(),
        retry_after
    );
    remote.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_tokens_gate_the_endpoint_and_bind_a_mode() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app; CREATE TABLE app.t (id int)")
        .await
        .unwrap();
    let tokens = format!(
        "{TOKEN_READ} read-only reader;{TOKEN_WRITE} read-write writer;{TOKEN_WRITE_ONLY} write-only writeonly"
    );
    let remote = remote(
        &scratch,
        Some(AuthMode::Bearer),
        &[("OWNPG_BEARER_TOKENS", tokens.as_str())],
    )
    .await;

    let (status, body, headers) = remote.tool("pg_health", None).await;
    assert_eq!(status, 401, "{body}");
    let challenge = headers
        .get("www-authenticate")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(challenge.starts_with("Bearer "), "{challenge}");
    assert!(
        challenge.contains("resource_metadata=\"http://127.0.0.1:"),
        "{challenge}"
    );
    assert!(
        challenge.contains("/.well-known/oauth-protected-resource/mcp\""),
        "{challenge}"
    );
    assert!(challenge.contains("scope=\"ownpg:read\""), "{challenge}");
    assert_eq!(body["error"], "invalid_request");

    let (status, body, _) = remote
        .tool("pg_health", Some("wrong-token-0123456789"))
        .await;
    assert_eq!(status, 401, "{body}");
    assert_eq!(body["error"], "invalid_token");

    let (status, body, _) = remote.tool("pg_health", Some(TOKEN_READ)).await;
    assert_eq!(status, 200, "{body}");
    assert_ne!(result_of(&body)["isError"], true, "{body}");

    let (status, body, headers) = remote.tool("pg_run_write", Some(TOKEN_READ)).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"], "insufficient_scope");
    assert!(
        headers
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("scope=\"ownpg:write\"")
    );

    let older = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": {"name": "pg_run_write", "arguments": {"sql": "DELETE FROM t"}}
    });
    let response = remote
        .client
        .post(format!("{}/mcp", remote.base))
        .header("MCP-Protocol-Version", "2025-11-25")
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .bearer_auth(TOKEN_READ)
        .body(older.to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result_of(&answer)["isError"], true, "{answer}");
    assert_eq!(
        result_of(&answer)["structuredContent"]["code"],
        "scope.insufficient"
    );

    let body = remote.call_body("pg_run_write", json!({"sql": "DELETE FROM t WHERE id = 1"}));
    let response = remote
        .post(&body)
        .header("Mcp-Name", "pg_run_write")
        .bearer_auth(TOKEN_WRITE)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_ne!(result_of(&answer)["isError"], true, "{answer}");

    let list = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "resources/list",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = remote
        .post(&list)
        .bearer_auth(TOKEN_WRITE_ONLY)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let refused: Value = response.json().await.unwrap();
    assert_eq!(refused["error"], "insufficient_scope");
    let response = remote
        .post(&list)
        .bearer_auth(TOKEN_READ)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let older_list = json!({"jsonrpc": "2.0", "id": 5, "method": "resources/list", "params": {}});
    let response = remote
        .client
        .post(format!("{}/mcp", remote.base))
        .header("MCP-Protocol-Version", "2025-11-25")
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .bearer_auth(TOKEN_WRITE_ONLY)
        .body(older_list.to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ownpg:read"),
        "{answer}"
    );

    let listen = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "subscriptions/listen",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {}
            },
            "notifications": {"resourcesListChanged": true}
        }
    });
    let response = remote
        .post(&listen)
        .bearer_auth(TOKEN_WRITE_ONLY)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let refused: Value = response.json().await.unwrap();
    assert_eq!(refused["error"], "insufficient_scope");
    assert!(
        refused["error_description"]
            .as_str()
            .unwrap()
            .contains("ownpg:read"),
        "{refused}"
    );

    let mut last = 401;
    for _ in 0..70 {
        let (status, _, _) = remote
            .tool("pg_health", Some("wrong-token-0123456789"))
            .await;
        last = status;
        if status == 429 {
            break;
        }
    }
    assert_eq!(last, 429, "guessing tokens is not throttled");
    remote.finish().await;
}

async fn guess_until_throttled(remote: &Remote, forwarded: &str) -> (u16, u16) {
    let body = remote.call_body("pg_health", json!({}));
    let mut first = None;
    let mut last = 401;
    for _ in 0..70 {
        let response = remote
            .post(&body)
            .header("Mcp-Name", "pg_health")
            .header("X-Forwarded-For", forwarded)
            .bearer_auth("wrong-token-0123456789")
            .send()
            .await
            .unwrap();
        last = response.status().as_u16();
        first.get_or_insert(last);
        if last == 429 {
            break;
        }
    }
    (first.unwrap_or(last), last)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarded_addresses_count_only_behind_a_trusted_proxy() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app")
        .await
        .unwrap();
    let tokens = format!("{TOKEN_READ} read-only reader");
    let behind_proxy = remote(
        &scratch,
        Some(AuthMode::Bearer),
        &[
            ("OWNPG_BEARER_TOKENS", tokens.as_str()),
            ("OWNPG_TRUSTED_PROXIES", "127.0.0.1"),
        ],
    )
    .await;
    assert_eq!(
        guess_until_throttled(&behind_proxy, "203.0.113.1").await,
        (401, 429)
    );
    assert_eq!(
        guess_until_throttled(&behind_proxy, "203.0.113.2, 127.0.0.1").await,
        (401, 429),
        "a second forwarded client gets its own bucket"
    );
    let (status, body, _) = behind_proxy.tool("pg_health", Some(TOKEN_READ)).await;
    assert_eq!(status, 200, "{body}");
    behind_proxy.finish().await;

    let exposed = remote(
        &scratch,
        Some(AuthMode::Bearer),
        &[("OWNPG_BEARER_TOKENS", tokens.as_str())],
    )
    .await;
    assert_eq!(
        guess_until_throttled(&exposed, "203.0.113.1").await,
        (401, 429)
    );
    let body = exposed.call_body("pg_health", json!({}));
    let response = exposed
        .post(&body)
        .header("Mcp-Name", "pg_health")
        .header("X-Forwarded-For", "203.0.113.2")
        .bearer_auth("wrong-token-0123456789")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        429,
        "a forged forwarded address must not open a fresh bucket"
    );
    exposed.finish().await;
}

struct Issuer {
    key: ring::signature::Ed25519KeyPair,
    jwks_url: String,
    task: tokio::task::JoinHandle<()>,
}

async fn issuer() -> Issuer {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let key = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key().as_ref());
    let document = json!({
        "keys": [{"kty": "OKP", "crv": "Ed25519", "kid": "k1", "use": "sig", "x": x}]
    });
    let app = axum::Router::new().route(
        "/jwks.json",
        axum::routing::get(move || {
            let document = document.clone();
            async move {
                (
                    [(axum::http::header::CACHE_CONTROL, "max-age=60")],
                    axum::Json(document),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Issuer {
        key,
        jwks_url: format!("http://{address}/jwks.json"),
        task,
    }
}

impl Issuer {
    fn token(&self, kid: &str, claims: Value) -> String {
        let header = json!({"alg": "EdDSA", "typ": "JWT", "kid": kid});
        let encode = |value: &Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
        };
        let signed = format!("{}.{}", encode(&header), encode(&claims));
        let signature = self.key.sign(signed.as_bytes());
        format!(
            "{signed}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.as_ref())
        )
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oauth_tokens_are_checked_against_the_issuer_keys() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app")
        .await
        .unwrap();
    let issuer = issuer().await;
    let remote = remote(
        &scratch,
        Some(AuthMode::Oauth),
        &[
            ("OWNPG_OAUTH_ISSUER", "https://issuer.test"),
            ("OWNPG_OAUTH_JWKS_URL", issuer.jwks_url.as_str()),
            ("OWNPG_PUBLIC_URL", "https://db.test/mcp"),
        ],
    )
    .await;
    let audience = "https://db.test/mcp";

    let metadata = remote
        .client
        .get(format!(
            "{}/.well-known/oauth-protected-resource",
            remote.base
        ))
        .send()
        .await
        .unwrap();
    let document: Value = metadata.json().await.unwrap();
    assert_eq!(
        document["authorization_servers"],
        json!(["https://issuer.test"])
    );
    assert_eq!(document["resource"], audience);

    let ready = remote
        .client
        .get(format!("{}/healthz/ready", remote.base))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);

    let good = issuer.token(
        "k1",
        json!({
            "iss": "https://issuer.test",
            "aud": audience,
            "sub": "alice",
            "exp": now() + 600,
            "scope": "ownpg:read"
        }),
    );
    let (status, body, _) = remote.tool("pg_health", Some(&good)).await;
    assert_eq!(status, 200, "{body}");
    assert_ne!(result_of(&body)["isError"], true, "{body}");

    let (status, body, headers) = remote.tool("pg_run_write", Some(&good)).await;
    assert_eq!(status, 403, "{body}");
    assert!(
        headers
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("insufficient_scope")
    );

    let other_audience = issuer.token(
        "k1",
        json!({"iss": "https://issuer.test", "aud": "https://other.test/mcp", "sub": "alice", "exp": now() + 600, "scope": "ownpg:read"}),
    );
    let (status, body, headers) = remote.tool("pg_health", Some(&other_audience)).await;
    assert_eq!(status, 401, "{body}");
    assert!(
        headers
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("resource_metadata=")
    );
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("audience")
    );

    let expired = issuer.token(
        "k1",
        json!({"iss": "https://issuer.test", "aud": audience, "sub": "alice", "exp": now() - 600, "scope": "ownpg:read"}),
    );
    let (status, body, _) = remote.tool("pg_health", Some(&expired)).await;
    assert_eq!(status, 401, "{body}");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("expired")
    );

    let unknown_key = issuer.token(
        "k2",
        json!({"iss": "https://issuer.test", "aud": audience, "sub": "alice", "exp": now() + 600, "scope": "ownpg:read"}),
    );
    let (status, body, _) = remote.tool("pg_health", Some(&unknown_key)).await;
    assert_eq!(status, 401, "{body}");

    let other_key = ring::signature::Ed25519KeyPair::from_pkcs8(
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let forged_issuer = Issuer {
        key: other_key,
        jwks_url: issuer.jwks_url.clone(),
        task: tokio::spawn(async {}),
    };
    let forged = forged_issuer.token(
        "k1",
        json!({"iss": "https://issuer.test", "aud": audience, "sub": "mallory", "exp": now() + 600, "scope": "ownpg:read"}),
    );
    let (status, body, _) = remote.tool("pg_health", Some(&forged)).await;
    assert_eq!(status, 401, "{body}");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("signature")
    );

    let (status, body, _) = remote.tool("pg_health", Some("not.a.jwt")).await;
    assert_eq!(status, 401, "{body}");

    let nameless = issuer.token(
        "k1",
        json!({"iss": "https://issuer.test", "aud": audience, "exp": now() + 600, "scope": "ownpg:read"}),
    );
    let (status, body, _) = remote.tool("pg_health", Some(&nameless)).await;
    assert_eq!(status, 401, "{body}");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("neither sub nor client_id"),
        "{body}"
    );
    forged_issuer.task.abort();
    remote.finish().await;
    issuer.task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_key_endpoint_fails_closed() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app")
        .await
        .unwrap();
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let jwks_url = format!("http://{address}/jwks.json");
    let remote = remote(
        &scratch,
        Some(AuthMode::Oauth),
        &[
            ("OWNPG_OAUTH_ISSUER", "https://issuer.test"),
            ("OWNPG_OAUTH_JWKS_URL", jwks_url.as_str()),
        ],
    )
    .await;
    let ready = remote
        .client
        .get(format!("{}/healthz/ready", remote.base))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 503);
    assert!(ready.text().await.unwrap().contains("key endpoint"));
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"alg": "EdDSA", "kid": "k1"}).to_string());
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({"iss": "https://issuer.test", "aud": format!("{}/mcp", remote.base), "exp": now() + 600})
            .to_string(),
    );
    let token = format!("{header}.{payload}.AAAA");
    let (status, body, _) = remote.tool("pg_health", Some(&token)).await;
    assert_eq!(status, 401, "{body}");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("key endpoint")
    );
    remote.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metrics_are_exported_to_the_configured_collector() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app")
        .await
        .unwrap();
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .with_test_writer()
        .try_init();
    let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&received);
    let sizes = Arc::clone(&bytes);
    let app = axum::Router::new().route(
        "/v1/metrics",
        axum::routing::post(move |body: axum::body::Bytes| {
            let counter = Arc::clone(&counter);
            let sizes = Arc::clone(&sizes);
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                sizes.fetch_add(body.len(), std::sync::atomic::Ordering::SeqCst);
                axum::http::StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let collector = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let endpoint = format!("http://{address}");
    let remote = remote(
        &scratch,
        None,
        &[("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str())],
    )
    .await;
    let (status, body, _) = remote.tool("pg_health", None).await;
    assert_eq!(status, 200, "{body}");
    let (status, _, _) = remote.tool("pg_run_write", None).await;
    assert_eq!(status, 200);
    remote.finish().await;
    assert!(
        received.load(std::sync::atomic::Ordering::SeqCst) >= 1,
        "no metrics payload reached the collector"
    );
    assert!(bytes.load(std::sync::atomic::Ordering::SeqCst) > 0);
    collector.abort();
}
