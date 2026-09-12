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
    for _ in 0..70 {
        let (status, _, headers) = remote.tool("pg_health", None).await;
        last = status;
        if status == 429 {
            retry_after = headers
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            break;
        }
    }
    assert_eq!(last, 429);
    assert!(retry_after.is_some_and(|seconds| seconds >= 1));
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
    let tokens = format!("{TOKEN_READ} read-only reader;{TOKEN_WRITE} read-write writer");
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

    let legacy = json!({
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
        .body(legacy.to_string())
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
    remote.finish().await;
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
