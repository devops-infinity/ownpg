pub mod auth;
pub mod jwks;
pub mod limit;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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
use super::stdio::StopReason;
use super::{Principal, Server};
use crate::config::{AuthMode, MCP_PATH, Settings};
use crate::error::{Error, ExitClass, Result};

pub const LIVE_PATH: &str = "/healthz/live";
pub const READY_PATH: &str = "/healthz/ready";
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[must_use]
pub fn probe_url(bind: SocketAddr, path: &str) -> String {
    let host = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    format!("http://{}{path}", SocketAddr::new(host, bind.port()))
}

pub async fn probe(bind: SocketAddr, path: &str, timeout: Duration) -> Result<String> {
    super::ensure_tls_provider();
    let url = probe_url(bind, path);
    let not_ready = |detail: String| Error::ServerNotReady {
        url: url.clone(),
        detail,
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| not_ready(format!("the probe client could not be built: {error}")))?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| not_ready(format!("no answer: {error}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| not_ready(format!("{status} with an unreadable body: {error}")))?;
    let body = body.trim().to_owned();
    if status.is_success() {
        Ok(body)
    } else {
        Err(not_ready(format!("{status} {body}")))
    }
}
pub const METADATA_PATH: &str = "/.well-known/oauth-protected-resource";
pub const HEADER_MCP_METHOD: &str = "mcp-method";
pub const HEADER_MCP_NAME: &str = "mcp-name";
pub const HEADER_MCP_SESSION: &str = "mcp-session-id";
const READINESS_CALLS_PER_MINUTE: u32 = 0;

pub struct Gatekeeper {
    pub server: Arc<Server>,
    pub authenticator: Authenticator,
    pub limiter: Limiter,
    pub readiness_limiter: Limiter,
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
    pub sessions: Arc<LocalSessionManager>,
    session_owners: std::sync::Mutex<std::collections::HashMap<String, String>>,
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
    fn challenge(&self, rejection: &Rejection, client: IpAddr) -> Response {
        let reason = rejection.error.unwrap_or(if rejection.status == 503 {
            "key_endpoint_unavailable"
        } else {
            "missing_token"
        });
        tracing::info!(%client, status = rejection.status, reason, detail = %rejection.description, "an HTTP request was refused");
        self.server.note_http_rejection(reason);
        let mut body = serde_json::Map::new();
        if let Some(error) = rejection.error {
            body.insert("error".to_owned(), serde_json::json!(error));
        }
        body.insert(
            "error_description".to_owned(),
            serde_json::json!(rejection.description),
        );
        if let Some(seconds) = rejection.retry_after_seconds {
            let mut response = (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::Value::Object(body)),
            )
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            return response;
        }
        let mut parts = vec![format!("resource_metadata=\"{}\"", self.metadata_url)];
        body.insert(
            "resource_metadata".to_owned(),
            serde_json::json!(self.metadata_url),
        );
        if let Some(error) = rejection.error {
            parts.push(format!("error=\"{error}\""));
            parts.push(format!(
                "error_description=\"{}\"",
                header_text(&rejection.description)
            ));
        }
        let scope = rejection
            .scope
            .clone()
            .unwrap_or_else(|| crate::tool_specs::SCOPE_READ.to_owned());
        parts.push(format!("scope=\"{scope}\""));
        let mut response = (
            StatusCode::from_u16(rejection.status).unwrap_or(StatusCode::UNAUTHORIZED),
            axum::Json(serde_json::Value::Object(body)),
        )
            .into_response();
        let value = HeaderValue::from_str(&format!("Bearer {}", parts.join(", ")))
            .unwrap_or_else(|_| HeaderValue::from_static("Bearer"));
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
        response
    }

    fn session_owner(&self, session: &str) -> Option<String> {
        self.session_owners
            .lock()
            .ok()
            .and_then(|owners| owners.get(session).cloned())
    }

    async fn bind_session(&self, session: &str, owner: &str) {
        let live: std::collections::HashSet<String> = self
            .sessions
            .sessions
            .read()
            .await
            .keys()
            .map(ToString::to_string)
            .collect();
        if let Ok(mut owners) = self.session_owners.lock() {
            owners.retain(|id, _| live.contains(id));
            owners.insert(session.to_owned(), owner.to_owned());
        }
    }

    fn unbind_session(&self, session: &str) {
        if let Ok(mut owners) = self.session_owners.lock() {
            owners.remove(session);
        }
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

    pub fn build(
        server: Arc<Server>,
        settings: &Settings,
        environment_tokens: Option<&str>,
        anonymous: Principal,
    ) -> Result<Self> {
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
        Ok(Self {
            server,
            authenticator,
            limiter: Limiter::new(http.rate_limit_per_minute.value),
            readiness_limiter: Limiter::new(READINESS_CALLS_PER_MINUTE),
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
            sessions: Arc::new(LocalSessionManager::default()),
            session_owners: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }
}

pub const RATE_LIMITED_CODE: i32 = -32000;

pub fn announce_proxy(purpose: &str) {
    for name in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            let host = value
                .rsplit_once('@')
                .map_or(value.as_str(), |(_, host)| host)
                .to_owned();
            tracing::info!(variable = name, proxy = %host, "{purpose} requests go through a proxy");
            return;
        }
    }
}

fn retry_after_seconds(wait: Duration) -> u64 {
    let whole = wait.as_secs();
    if wait.subsec_nanos() > 0 {
        whole.saturating_add(1)
    } else {
        whole.max(1)
    }
}

impl Gatekeeper {
    fn rate_limited(&self, wait: Duration, client: IpAddr) -> Response {
        tracing::info!(%client, wait_seconds = retry_after_seconds(wait), "a request was rate limited");
        self.server.note_http_rejection("rate_limited");
        too_many_requests(wait)
    }
}

fn too_many_requests(wait: Duration) -> Response {
    let seconds = retry_after_seconds(wait);
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "error": {
                "code": RATE_LIMITED_CODE,
                "message": format!("rate_limited: the limit of calls per minute was reached; retry after {seconds} seconds"),
                "data": {"retry_after_seconds": seconds},
            },
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

fn header_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' {
                c
            } else {
                ' '
            }
        })
        .collect()
}

fn principal_key(principal: &Principal) -> String {
    format!("{:?}:{}", principal.kind, principal.name)
}

fn method_not_allowed(older_client_sessions: bool) -> Response {
    let (allowed, text) = if older_client_sessions {
        (
            "GET, POST, DELETE",
            "only GET, POST, and DELETE are accepted on the MCP endpoint",
        )
    } else {
        ("POST", "only POST is accepted on the MCP endpoint")
    };
    let mut response = (StatusCode::METHOD_NOT_ALLOWED, text).into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static(allowed));
    response
}

fn session_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        "the session is not known to this caller; start a new session",
    )
        .into_response()
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
    State(gatekeeper): State<Arc<Gatekeeper>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let allowed = if gatekeeper.older_client_sessions {
        matches!(
            *request.method(),
            Method::POST | Method::GET | Method::DELETE
        )
    } else {
        *request.method() == Method::POST
    };
    if !allowed {
        tracing::debug!(method = %request.method(), "a request used a method the MCP endpoint does not accept");
        gatekeeper.server.note_http_rejection("method_not_allowed");
        return method_not_allowed(gatekeeper.older_client_sessions);
    }
    let client = client_address(peer, request.headers(), &gatekeeper.trusted_proxies);
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let principal = match gatekeeper
        .authenticator
        .authenticate(authorization.as_deref(), &gatekeeper.anonymous)
        .await
    {
        Ok(found) => found,
        Err(rejection) => {
            if let Err(wait) = gatekeeper.limiter.check(&format!("auth-fail:{client}")) {
                return gatekeeper.rate_limited(wait, client);
            }
            return gatekeeper.challenge(&rejection, client);
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
        && let Some(route) = gatekeeper.server.route(name)
        && !principal.allows(route.spec.scope)
    {
        return gatekeeper.challenge(&Rejection::insufficient(route.spec.scope), client);
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
    ) && !principal.allows(crate::tool_specs::SCOPE_READ)
    {
        return gatekeeper.challenge(
            &Rejection::insufficient(crate::tool_specs::SCOPE_READ),
            client,
        );
    }
    let owner = principal_key(&principal);
    let bucket = if gatekeeper.authenticator.requires_token() {
        format!("principal:{owner}")
    } else {
        format!("addr:{client}")
    };
    if let Err(wait) = gatekeeper.limiter.check(&bucket) {
        return gatekeeper.rate_limited(wait, client);
    }
    let session = request
        .headers()
        .get(HEADER_MCP_SESSION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if gatekeeper.older_client_sessions
        && let Some(id) = &session
        && gatekeeper.session_owner(id).as_deref() != Some(owner.as_str())
    {
        tracing::info!(%client, principal = %principal.name, "a request named a session that belongs to another caller or does not exist");
        gatekeeper.server.note_http_rejection("session_not_found");
        return session_not_found();
    }
    let closing = *request.method() == Method::DELETE;
    request.extensions_mut().insert(principal);
    let mut response = next.run(request).await;
    if gatekeeper.older_client_sessions {
        match &session {
            None => {
                if let Some(created) = response
                    .headers()
                    .get(HEADER_MCP_SESSION)
                    .and_then(|value| value.to_str().ok())
                {
                    gatekeeper.bind_session(created, &owner).await;
                }
            }
            Some(id) if closing && response.status().is_success() => {
                gatekeeper.unbind_session(id);
            }
            Some(_) => {}
        }
    }
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
    State(gatekeeper): State<Arc<Gatekeeper>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Response {
    let client = client_address(peer, &headers, &gatekeeper.trusted_proxies);
    if let Err(wait) = gatekeeper
        .readiness_limiter
        .check(&format!("ready:{client}"))
    {
        return gatekeeper.rate_limited(wait, client);
    }
    if !gatekeeper.server.engine().is_alive().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready: PostgreSQL is not reachable",
        )
            .into_response();
    }
    if gatekeeper.server.audit().on_failure() == crate::config::AuditFailure::RefuseAll
        && let Err(error) = gatekeeper.server.audit_admits(true).await
    {
        tracing::warn!(%error, "the audit log is not ready");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready: the audit log cannot be written",
        )
            .into_response();
    }
    if let Err(detail) = gatekeeper.authenticator.ready().await {
        tracing::warn!(%detail, "the key endpoint is not ready");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready: the key endpoint is not reachable",
        )
            .into_response();
    }
    (StatusCode::OK, "ready").into_response()
}

async fn metadata(State(gatekeeper): State<Arc<Gatekeeper>>) -> Response {
    let mut response = axum::Json(gatekeeper.metadata()).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

pub fn router(gatekeeper: Arc<Gatekeeper>, settings: &Settings) -> Router {
    let server = Arc::clone(&gatekeeper.server);
    let http = &settings.http;
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(http.older_client_sessions.value)
        .with_json_response(true)
        .with_cancellation_token(gatekeeper.cancel.clone())
        .with_allowed_hosts(http.allowed_hosts.value.clone())
        .with_allowed_origins(http.allowed_origins.value.clone())
        .with_max_request_body_bytes(http.body_cap.value);
    let mcp = StreamableHttpService::new(
        move || Ok(Arc::clone(&server)),
        Arc::clone(&gatekeeper.sessions),
        config,
    );
    let body_timeout = gatekeeper.body_timeout;
    Router::new()
        .route_service(MCP_PATH, mcp)
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&gatekeeper),
            guard,
        ))
        .route(LIVE_PATH, get(live))
        .route(READY_PATH, get(ready))
        .route(METADATA_PATH, get(metadata))
        .route(&format!("{METADATA_PATH}{MCP_PATH}"), get(metadata))
        .layer(tower_http::timeout::RequestBodyTimeoutLayer::new(
            body_timeout,
        ))
        .with_state(gatekeeper)
}

pub async fn shutdown_signal() -> StopReason {
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
        () = ctrl_c => StopReason::Interrupt,
        () = terminate => StopReason::Terminate,
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
    gatekeeper: Arc<Gatekeeper>,
    shutdown: impl std::future::Future<Output = StopReason> + Send + 'static,
    deadline: Duration,
) -> Result<ExitClass> {
    let stop = CancellationToken::new();
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let permits = Arc::new(tokio::sync::Semaphore::new(gatekeeper.max_connections));
    let mut builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    builder
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(gatekeeper.header_timeout)
        .keep_alive(true);
    let builder = Arc::new(builder);
    let limiter_sweeper = {
        let gatekeeper = Arc::clone(&gatekeeper);
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        gatekeeper.limiter.forget_idle();
                        gatekeeper.readiness_limiter.forget_idle();
                    },
                    () = stop.cancelled() => break,
                }
            }
        })
    };
    tracing::info!(address = %listening.local_addr, "listening for Streamable HTTP");
    let mut shutdown = std::pin::pin!(shutdown);
    let listener = listening.listener;
    let reason = loop {
        let permit = tokio::select! {
            permit = permits.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => break StopReason::Terminate,
            },
            reason = &mut shutdown => break reason,
        };
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            reason = &mut shutdown => break reason,
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
    };
    tracing::info!("stopping; draining in-flight calls (a second signal exits at once)");
    stop.cancel();
    limiter_sweeper.abort();
    drop(listener);
    let drained = tokio::select! {
        drained = tokio::time::timeout(deadline, graceful.shutdown()) => drained.is_ok(),
        _ = shutdown_signal() => {
            tracing::warn!("a second signal arrived; exiting at once");
            gatekeeper.cancel.cancel();
            return Ok(super::stdio::exit_for(reason));
        }
    };
    if !drained {
        tracing::warn!(
            "in-flight calls did not finish within {} ms; closing them",
            deadline.as_millis()
        );
    }
    gatekeeper.cancel.cancel();
    gatekeeper.server.shutdown().await;
    Ok(super::stdio::exit_for(reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wildcard_bind_is_probed_on_loopback_of_the_same_family() {
        let parse = |text: &str| text.parse::<SocketAddr>().unwrap();
        assert_eq!(
            probe_url(parse("0.0.0.0:8765"), READY_PATH),
            "http://127.0.0.1:8765/healthz/ready"
        );
        assert_eq!(
            probe_url(parse("[::]:8765"), LIVE_PATH),
            "http://[::1]:8765/healthz/live"
        );
        assert_eq!(
            probe_url(parse("10.0.0.5:9000"), READY_PATH),
            "http://10.0.0.5:9000/healthz/ready"
        );
    }

    async fn health_stub(status: StatusCode, body: &'static str) -> SocketAddr {
        let app = Router::new().route(READY_PATH, get(move || async move { (status, body) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        address
    }

    #[tokio::test]
    async fn the_probe_passes_on_200_and_names_the_reason_on_503() {
        let ready = health_stub(StatusCode::OK, "ready").await;
        assert_eq!(
            probe(ready, READY_PATH, PROBE_TIMEOUT).await.unwrap(),
            "ready"
        );
        let waiting = health_stub(
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready: PostgreSQL is not reachable",
        )
        .await;
        let error = probe(waiting, READY_PATH, PROBE_TIMEOUT).await.unwrap_err();
        assert_eq!(error.id(), crate::error::ErrorId::ServerNotReady);
        assert_eq!(error.exit_class(), ExitClass::Runtime);
        assert!(
            error.to_string().contains("PostgreSQL is not reachable"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn the_probe_fails_when_nothing_listens() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = probe(address, READY_PATH, PROBE_TIMEOUT).await.unwrap_err();
        assert_eq!(error.id(), crate::error::ErrorId::ServerNotReady);
        assert!(error.to_string().contains("no answer"), "{error}");
    }

    #[tokio::test]
    async fn a_rate_limited_response_carries_a_schema_valid_id() {
        use http_body_util::BodyExt;
        let response = too_many_requests(Duration::from_secs(1));
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = &value["id"];
        assert!(
            id.is_string() || id.is_i64() || id.is_u64(),
            "the 2026-07-28 wire schema requires id to be a string or an integer, never null: {id}"
        );
    }

    #[test]
    fn the_retry_delay_rounds_up_to_whole_seconds() {
        assert_eq!(retry_after_seconds(Duration::from_millis(1)), 1);
        assert_eq!(retry_after_seconds(Duration::from_millis(1_900)), 2);
        assert_eq!(retry_after_seconds(Duration::from_secs(3)), 3);
        assert_eq!(retry_after_seconds(Duration::ZERO), 1);
    }

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
