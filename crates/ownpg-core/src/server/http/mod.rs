pub mod auth;
pub mod jwks;
pub mod limit;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use self::auth::{Authenticator, Rejection};
use self::limit::Limiter;
use super::{Principal, Server};
use crate::config::{AuthMode, MCP_PATH, Settings};
use crate::error::{Error, ExitClass, Result};

pub const LIVE_PATH: &str = "/healthz/live";
pub const READY_PATH: &str = "/healthz/ready";
pub const METADATA_PATH: &str = "/.well-known/oauth-protected-resource";
pub const HEADER_MCP_METHOD: &str = "mcp-method";
pub const HEADER_MCP_NAME: &str = "mcp-name";

pub struct Gatekeeper {
    pub server: Arc<Server>,
    pub authenticator: Authenticator,
    pub limiter: Limiter,
    pub anonymous: Principal,
    pub older_client_sessions: bool,
    pub metadata_url: String,
    pub public_url: String,
    pub authorization_server: Option<String>,
    pub cancel: CancellationToken,
    pub trusted_proxies: Vec<IpAddr>,
    pub max_connections: usize,
    pub header_timeout: Duration,
    pub body_timeout: Duration,
}

impl std::fmt::Debug for Gatekeeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gatekeeper")
            .field("public_url", &self.public_url)
            .field("older_client_sessions", &self.older_client_sessions)
            .finish_non_exhaustive()
    }
}

impl Gatekeeper {
    fn challenge(&self, rejection: &Rejection) -> Response {
        let mut parts = vec![format!("resource_metadata=\"{}\"", self.metadata_url)];
        if self.authenticator.requires_token() {
            parts.push(format!("error=\"{}\"", rejection.error));
            parts.push(format!(
                "error_description=\"{}\"",
                rejection.description.replace('"', "'")
            ));
        }
        let scope = rejection
            .scope
            .clone()
            .unwrap_or_else(|| crate::groups::SCOPE_READ.to_owned());
        parts.push(format!("scope=\"{scope}\""));
        let body = serde_json::json!({
            "error": rejection.error,
            "error_description": rejection.description,
            "resource_metadata": self.metadata_url,
        });
        let mut response = (
            StatusCode::from_u16(rejection.status).unwrap_or(StatusCode::UNAUTHORIZED),
            axum::Json(body),
        )
            .into_response();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", parts.join(", "))) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        response
    }

    fn metadata(&self) -> serde_json::Value {
        let mut document = serde_json::Map::new();
        document.insert("resource".to_owned(), serde_json::json!(self.public_url));
        document.insert("resource_name".to_owned(), serde_json::json!("OwnPG"));
        document.insert(
            "bearer_methods_supported".to_owned(),
            serde_json::json!(["header"]),
        );
        document.insert(
            "scopes_supported".to_owned(),
            serde_json::json!(auth::ALL_SCOPES),
        );
        if let Some(issuer) = &self.authorization_server {
            document.insert(
                "authorization_servers".to_owned(),
                serde_json::json!([issuer]),
            );
        }
        serde_json::Value::Object(document)
    }
}

pub const RATE_LIMITED_CODE: i32 = -32000;

fn too_many(wait: Duration) -> Response {
    let seconds = wait.as_secs().max(1);
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": serde_json::Value::Null,
            "error": {
                "code": RATE_LIMITED_CODE,
                "message": format!("rate_limited: the limit of calls per minute was reached; retry after {seconds} seconds"),
                "data": {"retry_after_seconds": seconds},
            },
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&wait.as_secs().max(1).to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

fn method_not_allowed() -> Response {
    let mut response = (
        StatusCode::METHOD_NOT_ALLOWED,
        "only POST is accepted on the MCP endpoint",
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST"));
    response
}

#[must_use]
pub fn client_address(
    peer: SocketAddr,
    headers: &axum::http::HeaderMap,
    trusted_proxies: &[IpAddr],
) -> IpAddr {
    if !trusted_proxies.contains(&peer.ip()) {
        return peer.ip();
    }
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter_map(|item| {
            item.trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .ok()
                .or_else(|| item.parse::<SocketAddr>().ok().map(|address| address.ip()))
        })
        .rev()
        .find(|address| !trusted_proxies.contains(address))
        .unwrap_or_else(|| peer.ip())
}

pub async fn guard(
    State(gate): State<Arc<Gatekeeper>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if !gate.older_client_sessions && matches!(*request.method(), Method::GET | Method::DELETE) {
        return method_not_allowed();
    }
    if !matches!(
        *request.method(),
        Method::POST | Method::GET | Method::DELETE
    ) {
        return method_not_allowed();
    }
    let client = client_address(peer, request.headers(), &gate.trusted_proxies);
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let (principal, key) = match gate
        .authenticator
        .authenticate(authorization.as_deref(), &gate.anonymous)
        .await
    {
        Ok(found) => found,
        Err(rejection) => {
            if let Err(wait) = gate.limiter.check(&format!("auth-fail:{client}")) {
                return too_many(wait);
            }
            return gate.challenge(&rejection);
        }
    };
    let method_header = request
        .headers()
        .get(HEADER_MCP_METHOD)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if method_header == "tools/call"
        && let Some(name) = request
            .headers()
            .get(HEADER_MCP_NAME)
            .and_then(|value| value.to_str().ok())
        && let Some(route) = gate.server.route(name)
        && !principal.allows(route.spec.scope)
    {
        return gate.challenge(&Rejection::insufficient(route.spec.scope));
    }
    if matches!(
        method_header.as_str(),
        "resources/list"
            | "resources/read"
            | "resources/templates/list"
            | "resources/subscribe"
            | "resources/unsubscribe"
            | "subscriptions/listen"
            | "prompts/list"
            | "prompts/get"
            | "completion/complete"
    ) && !principal.allows(crate::groups::SCOPE_READ)
    {
        return gate.challenge(&Rejection::insufficient(crate::groups::SCOPE_READ));
    }
    let bucket = if key.is_empty() {
        format!("addr:{client}")
    } else {
        format!("token:{key}")
    };
    if let Err(wait) = gate.limiter.check(&bucket) {
        return too_many(wait);
    }
    request.extensions_mut().insert(principal);
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

async fn live() -> Response {
    (StatusCode::OK, "live").into_response()
}

async fn ready(
    State(gate): State<Arc<Gatekeeper>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Response {
    let client = client_address(peer, &headers, &gate.trusted_proxies);
    if let Err(wait) = gate.limiter.check(&format!("ready:{client}")) {
        return too_many(wait);
    }
    if !gate.server.engine().is_alive().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready: PostgreSQL is not reachable",
        )
            .into_response();
    }
    if let Err(detail) = gate.authenticator.ready().await {
        tracing::warn!(%detail, "the key endpoint is not ready");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready: the key endpoint is not reachable",
        )
            .into_response();
    }
    (StatusCode::OK, "ready").into_response()
}

async fn metadata(State(gate): State<Arc<Gatekeeper>>) -> Response {
    let mut response = axum::Json(gate.metadata()).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

pub fn router(gate: Arc<Gatekeeper>, settings: &Settings) -> Router {
    let server = Arc::clone(&gate.server);
    let http = &settings.http;
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(http.older_client_sessions.value)
        .with_json_response(true)
        .with_cancellation_token(gate.cancel.clone())
        .with_allowed_hosts(http.allowed_hosts.value.clone())
        .with_allowed_origins(http.allowed_origins.value.clone())
        .with_max_request_body_bytes(http.body_cap.value);
    let mcp = StreamableHttpService::new(
        move || Ok(Arc::clone(&server)),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Router::new()
        .route_service(MCP_PATH, mcp)
        .route_layer(middleware::from_fn_with_state(Arc::clone(&gate), guard))
        .route_layer(tower_http::timeout::RequestBodyTimeoutLayer::new(
            gate.body_timeout,
        ))
        .route(LIVE_PATH, get(live))
        .route(READY_PATH, get(ready))
        .route(METADATA_PATH, get(metadata))
        .route(&format!("{METADATA_PATH}{MCP_PATH}"), get(metadata))
        .with_state(gate)
}

pub fn gatekeeper(
    server: Arc<Server>,
    settings: &Settings,
    environment_tokens: Option<&str>,
    anonymous: Principal,
) -> Result<Gatekeeper> {
    let http = &settings.http;
    if !http.is_loopback() && http.auth.mode() == AuthMode::None {
        return Err(Error::ConfigInvalid {
            setting: "bind".to_owned(),
            value: http.bind.value.to_string(),
            detail:
                "a bind address outside the loopback interface needs --auth bearer or --auth oauth"
                    .to_owned(),
        });
    }
    let authenticator = Authenticator::build(settings, environment_tokens)?;
    let authorization_server = match &http.auth {
        crate::config::AuthSettings::Oauth(oauth) => Some(oauth.issuer.value.clone()),
        _ => None,
    };
    Ok(Gatekeeper {
        server,
        authenticator,
        limiter: Limiter::new(http.rate_limit_per_minute.value),
        anonymous,
        older_client_sessions: http.older_client_sessions.value,
        metadata_url: http.metadata_url(),
        public_url: http.public_url.value.clone(),
        authorization_server,
        cancel: CancellationToken::new(),
        trusted_proxies: http.trusted_proxies.value.clone(),
        max_connections: usize::try_from(http.max_connections.value).unwrap_or(usize::MAX),
        header_timeout: crate::config::http::DEFAULT_HEADER_TIMEOUT,
        body_timeout: crate::config::http::DEFAULT_BODY_TIMEOUT,
    })
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(%error, "the interrupt signal could not be watched");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "the terminate signal could not be watched");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[derive(Debug)]
pub struct Listening {
    pub local_addr: SocketAddr,
    listener: tokio::net::TcpListener,
}

impl Listening {
    pub async fn bind(address: SocketAddr) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|error| Error::ProtocolFailed {
                detail: format!("{address} could not be bound: {error}"),
            })?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| Error::ProtocolFailed {
                detail: format!("the bound address could not be read: {error}"),
            })?;
        Ok(Self {
            local_addr,
            listener,
        })
    }
}

pub async fn serve(
    listening: Listening,
    router: Router,
    gate: Arc<Gatekeeper>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    deadline: Duration,
) -> Result<ExitClass> {
    let stop = CancellationToken::new();
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let permits = Arc::new(tokio::sync::Semaphore::new(gate.max_connections));
    let mut builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    builder
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(gate.header_timeout)
        .keep_alive(true);
    let builder = Arc::new(builder);
    let sweeper = {
        let limiter_gate = Arc::clone(&gate);
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = tick.tick() => limiter_gate.limiter.forget_idle(),
                    () = stop.cancelled() => break,
                }
            }
        })
    };
    tracing::info!(address = %listening.local_addr, "listening for Streamable HTTP");
    let mut shutdown = std::pin::pin!(shutdown);
    let listener = listening.listener;
    loop {
        let permit = tokio::select! {
            permit = permits.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => break,
            },
            () = &mut shutdown => break,
        };
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            () = &mut shutdown => break,
        };
        let (socket, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(%error, "a connection could not be accepted");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let app = router.clone();
        let service = tower::util::ServiceExt::map_request(
            app,
            move |mut request: axum::http::Request<hyper::body::Incoming>| {
                request.extensions_mut().insert(ConnectInfo(peer));
                request.map(Body::new)
            },
        );
        let hyper_service = hyper_util::service::TowerToHyperService::new(service);
        let io = hyper_util::rt::TokioIo::new(socket);
        let connection = builder.serve_connection_with_upgrades(io, hyper_service);
        let watched = graceful.watch(connection.into_owned());
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) = watched.await {
                tracing::debug!(%error, "a connection ended with an error");
            }
        });
    }
    tracing::info!("shutting down; draining in-flight calls");
    stop.cancel();
    sweeper.abort();
    drop(listener);
    if tokio::time::timeout(deadline, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!(
            "in-flight calls did not finish within {} ms; closing them",
            deadline.as_millis()
        );
    }
    gate.cancel.cancel();
    gate.server.shutdown().await;
    Ok(ExitClass::Success)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(forwarded: &[&str]) -> axum::http::HeaderMap {
        let mut map = axum::http::HeaderMap::new();
        for value in forwarded {
            map.append("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn forwarded_addresses_are_read_only_from_trusted_proxies() {
        let proxy: IpAddr = "10.0.0.1".parse().unwrap();
        let stranger: SocketAddr = "198.51.100.7:4000".parse().unwrap();
        let via_proxy: SocketAddr = "10.0.0.1:4000".parse().unwrap();
        let forwarded = headers(&["203.0.113.9, 10.0.0.1"]);
        assert_eq!(
            client_address(stranger, &forwarded, &[proxy]),
            stranger.ip(),
            "a peer that is not a proxy cannot name another client"
        );
        assert_eq!(
            client_address(via_proxy, &forwarded, &[proxy]),
            "203.0.113.9".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_address(via_proxy, &headers(&["[2001:db8::5]:443"]), &[proxy]),
            "2001:db8::5".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_address(via_proxy, &headers(&["garbage", "10.0.0.1"]), &[proxy]),
            via_proxy.ip(),
            "a header naming only proxies or junk falls back to the peer"
        );
        assert_eq!(
            client_address(via_proxy, &headers(&["1.1.1.1", "9.9.9.9"]), &[proxy]),
            "9.9.9.9".parse::<IpAddr>().unwrap(),
            "the rightmost non-proxy entry is the client"
        );
        assert_eq!(client_address(via_proxy, &forwarded, &[]), via_proxy.ip());
    }
}
