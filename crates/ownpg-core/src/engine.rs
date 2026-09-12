use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, TryStreamExt};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_postgres::SimpleQueryMessage;
use tokio_postgres::types::{ToSql, Type};

use crate::config::Settings;
use crate::connect::role::RoleProfile;
use crate::connect::ssh::Hints;
use crate::connect::{
    Connector, Session, SessionInfo, describe_sqlstate, pooled_transaction_prefix,
};
use crate::error::{Error, Result};
use crate::shape::{Caps, Collector, Column, ResultSet};

pub const CURSOR_CAP: usize = 8;
const SAVEPOINT: &str = "ownpg_call";

pub trait CatalogParam: ToSql + Sync {
    fn pg_type(&self) -> Type;
}

macro_rules! catalog_param {
    ($($rust:ty => $pg:expr),* $(,)?) => {
        $(impl CatalogParam for $rust {
            fn pg_type(&self) -> Type {
                $pg
            }
        })*
    };
}

catalog_param! {
    String => Type::TEXT,
    &str => Type::TEXT,
    i64 => Type::INT8,
    i32 => Type::INT4,
    i16 => Type::INT2,
    u32 => Type::OID,
    bool => Type::BOOL,
    Vec<String> => Type::TEXT_ARRAY,
}

impl<T: CatalogParam> CatalogParam for Option<T>
where
    Option<T>: ToSql + Sync,
{
    fn pg_type(&self) -> Type {
        self.as_ref().map_or(Type::TEXT, CatalogParam::pg_type)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
    pub server_version_num: i32,
}

impl Features {
    #[must_use]
    pub const fn from_version(server_version_num: i32) -> Self {
        Self { server_version_num }
    }

    #[must_use]
    pub const fn supports_stat_io(self) -> bool {
        self.server_version_num >= 160_000
    }

    #[must_use]
    pub const fn supports_merge(self) -> bool {
        self.server_version_num >= 150_000
    }

    #[must_use]
    pub const fn supports_merge_returning(self) -> bool {
        self.server_version_num >= 170_000
    }

    #[must_use]
    pub const fn supports_transaction_timeout(self) -> bool {
        self.server_version_num >= 170_000
    }

    #[must_use]
    pub const fn supports_stat_checkpointer(self) -> bool {
        self.server_version_num >= 170_000
    }

    #[must_use]
    pub const fn supports_returning_old_new(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub const fn supports_not_enforced_constraints(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub const fn supports_virtual_generated_columns(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub const fn supports_nulls_not_distinct(self) -> bool {
        self.server_version_num >= 150_000
    }

    #[must_use]
    pub const fn supports_without_overlaps(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub const fn supports_security_invoker(self) -> bool {
        self.server_version_num >= 150_000
    }

    #[must_use]
    pub const fn supports_maintain_privilege(self) -> bool {
        self.server_version_num >= 170_000
    }

    #[must_use]
    pub fn as_map(self) -> BTreeMap<&'static str, bool> {
        BTreeMap::from([
            ("pg_stat_io", self.supports_stat_io()),
            ("merge", self.supports_merge()),
            ("merge_returning", self.supports_merge_returning()),
            ("transaction_timeout", self.supports_transaction_timeout()),
            ("pg_stat_checkpointer", self.supports_stat_checkpointer()),
            ("returning_old_new", self.supports_returning_old_new()),
            (
                "not_enforced_constraints",
                self.supports_not_enforced_constraints(),
            ),
            (
                "virtual_generated_columns",
                self.supports_virtual_generated_columns(),
            ),
            ("nulls_not_distinct", self.supports_nulls_not_distinct()),
            ("without_overlaps", self.supports_without_overlaps()),
            ("maintain_privilege", self.supports_maintain_privilege()),
        ])
    }
}

#[derive(Debug)]
struct Cursor {
    name: String,
    principal: String,
    columns: Vec<Column>,
    estimate: Option<i64>,
    expires_at: Instant,
    pending: VecDeque<Vec<Option<String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleState {
    Open,
    Expired,
    Committed,
    RolledBack,
    Lost,
}

impl HandleState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Expired => "expired",
            Self::Committed => "committed",
            Self::RolledBack => "rolled_back",
            Self::Lost => "lost",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct HandleInfo {
    pub id: String,
    pub state: HandleState,
    pub principal: String,
    pub savepoints: Vec<String>,
    pub statements: u64,
    pub expires_in_seconds: u64,
}

#[derive(Debug)]
struct WriteHandle {
    id: String,
    principal: String,
    savepoints: Vec<String>,
    statements: u64,
    expires_at: Instant,
}

pub struct SessionManager {
    connector: Arc<Connector>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager").finish_non_exhaustive()
    }
}

impl deadpool::managed::Manager for SessionManager {
    type Type = Session;
    type Error = Error;

    async fn create(&self) -> Result<Session> {
        self.connector.connect().await
    }

    async fn recycle(
        &self,
        session: &mut Session,
        _metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Error> {
        if session.is_alive().await {
            Ok(())
        } else {
            Err(deadpool::managed::RecycleError::Message(
                "the pooled connection is closed".into(),
            ))
        }
    }
}

pub type Pool = deadpool::managed::Pool<SessionManager>;
type PooledSession = deadpool::managed::Object<SessionManager>;

fn pool_error(error: deadpool::managed::PoolError<Error>) -> Error {
    match error {
        deadpool::managed::PoolError::Backend(error) => error,
        other => Error::ProtocolFailed {
            detail: format!("no pooled connection was available: {other}"),
        },
    }
}

enum Conn {
    Owned(Session),
    Pooled(PooledSession),
}

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owned(session) => f.debug_tuple("Owned").field(&session.info.target).finish(),
            Self::Pooled(object) => f.debug_tuple("Pooled").field(&object.info.target).finish(),
        }
    }
}

impl Conn {
    fn session(&self) -> &Session {
        match self {
            Self::Owned(session) => session,
            Self::Pooled(object) => object,
        }
    }

    fn client(&self) -> &tokio_postgres::Client {
        &self.session().client
    }

    async fn is_alive(&self) -> bool {
        self.session().is_alive().await
    }
}

#[derive(Debug)]
struct Lane {
    conn: Conn,
    in_read_transaction: bool,
    cursors: BTreeMap<String, Cursor>,
    next_cursor: u64,
}

impl Lane {
    fn new(session: Session) -> Self {
        Self::from_conn(Conn::Owned(session))
    }

    const fn from_conn(conn: Conn) -> Self {
        Self {
            conn,
            in_read_transaction: false,
            cursors: BTreeMap::new(),
            next_cursor: 0,
        }
    }

    fn forget_state(&mut self) {
        self.in_read_transaction = false;
        self.cursors.clear();
    }
}

#[derive(Debug)]
struct Primary {
    lane: Lane,
    write_handle: Option<WriteHandle>,
    closed: BTreeMap<String, HandleState>,
}

#[derive(Debug)]
struct PooledHandle {
    lane: Lane,
    handle: WriteHandle,
}

#[derive(Debug, Default)]
struct PooledHandles {
    open: HashMap<String, PooledHandle>,
    closed: BTreeMap<String, HandleState>,
}

tokio::task_local! {
    pub static CALL_ID: u64;
}

#[derive(Default)]
struct Running {
    next_seq: u64,
    cancel_tokens: HashMap<u64, Vec<(u64, tokio_postgres::CancelToken)>>,
}

struct Tracked<'a> {
    engine: &'a Engine,
    key: Option<(u64, u64)>,
}

impl Drop for Tracked<'_> {
    fn drop(&mut self) {
        let Some((call, seq)) = self.key else {
            return;
        };
        let Ok(mut running) = self.engine.running.lock() else {
            return;
        };
        if let Some(cancel_tokens) = running.cancel_tokens.get_mut(&call) {
            cancel_tokens.retain(|(held, _)| *held != seq);
            if cancel_tokens.is_empty() {
                running.cancel_tokens.remove(&call);
            }
        }
    }
}

pub struct Engine {
    settings: Arc<Settings>,
    connector: Arc<Connector>,
    primary: Mutex<Primary>,
    secondary: Mutex<Option<Lane>>,
    pool: Option<Pool>,
    pinned: Mutex<Vec<Lane>>,
    pooled_handles: Mutex<PooledHandles>,
    running: std::sync::Mutex<Running>,
    role: tokio::sync::OnceCell<RoleProfile>,
    features: Features,
    transaction_prefix: Option<String>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("features", &self.features)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorSummary {
    pub id: String,
    pub expires_in_seconds: u64,
}

impl Engine {
    pub async fn start(settings: Arc<Settings>, hints: Hints) -> Result<Self> {
        Self::start_with(settings, hints, false).await
    }

    pub async fn start_pooled(settings: Arc<Settings>, hints: Hints) -> Result<Self> {
        Self::start_with(settings, hints, true).await
    }

    async fn start_with(settings: Arc<Settings>, hints: Hints, pooled: bool) -> Result<Self> {
        let connector = Arc::new(Connector::new(Arc::clone(&settings)).with_ssh_hints(hints));
        let session = connector.connect().await?;
        let features = Features::from_version(session.info.server_version_num);
        let transaction_prefix =
            pooled_transaction_prefix(&settings, session.info.server_version_num);
        let pool = if pooled {
            let size = usize::try_from(settings.http.pool_size.value).unwrap_or(4);
            let pool = Pool::builder(SessionManager {
                connector: Arc::clone(&connector),
            })
            .max_size(size)
            .wait_timeout(Some(settings.limits.statement_timeout.value))
            .create_timeout(Some(settings.connection.connect_timeout.value))
            .recycle_timeout(Some(Duration::from_secs(5)))
            .runtime(deadpool::Runtime::Tokio1)
            .build()
            .map_err(|error| Error::ProtocolFailed {
                detail: format!("the connection pool could not be built: {error}"),
            })?;
            let warm = pool.get().await.map_err(pool_error)?;
            drop(warm);
            Some(pool)
        } else {
            None
        };
        let engine = Self {
            settings,
            connector,
            primary: Mutex::new(Primary {
                lane: Lane::new(session),
                write_handle: None,
                closed: BTreeMap::new(),
            }),
            secondary: Mutex::new(None),
            pool,
            pinned: Mutex::new(Vec::new()),
            pooled_handles: Mutex::new(PooledHandles::default()),
            running: std::sync::Mutex::new(Running::default()),
            role: tokio::sync::OnceCell::new(),
            features,
            transaction_prefix,
        };
        if engine.settings.strict_role.value {
            engine.role().await?.enforce(true)?;
        }
        Ok(engine)
    }

    #[must_use]
    pub fn pool_size(&self) -> Option<usize> {
        self.pool.as_ref().map(|pool| pool.status().max_size)
    }

    async fn checkout(&self) -> Result<Lane> {
        let Some(pool) = &self.pool else {
            return Err(Error::ProtocolFailed {
                detail: "no connection pool is open".to_owned(),
            });
        };
        let object = pool.get().await.map_err(pool_error)?;
        Ok(Lane::from_conn(Conn::Pooled(object)))
    }

    #[must_use]
    pub fn settings(&self) -> &Arc<Settings> {
        &self.settings
    }

    #[must_use]
    pub const fn features(&self) -> Features {
        self.features
    }

    pub async fn info(&self) -> SessionInfo {
        self.primary.lock().await.lane.conn.session().info.clone()
    }

    pub async fn role(&self) -> Result<&RoleProfile> {
        self.role
            .get_or_try_init(|| async {
                let primary = self.primary.lock().await;
                RoleProfile::load(primary.lane.conn.client()).await
            })
            .await
    }

    pub async fn is_alive(&self) -> bool {
        self.primary.lock().await.lane.conn.is_alive().await
    }

    pub async fn cancel_call(&self, call: u64) -> Result<()> {
        let cancel_tokens = self
            .running
            .lock()
            .map_err(|_| Error::ProtocolFailed {
                detail: "the running-call lock is poisoned".to_owned(),
            })?
            .cancel_tokens
            .get(&call)
            .map(|cancel_tokens| {
                cancel_tokens
                    .iter()
                    .map(|(_, token)| token.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.send_cancel(cancel_tokens).await
    }

    pub async fn cancel_running_statements(&self) -> Result<()> {
        let cancel_tokens = self
            .running
            .lock()
            .map_err(|_| Error::ProtocolFailed {
                detail: "the running-call lock is poisoned".to_owned(),
            })?
            .cancel_tokens
            .values()
            .flat_map(|cancel_tokens| cancel_tokens.iter().map(|(_, token)| token.clone()))
            .collect();
        self.send_cancel(cancel_tokens).await
    }

    async fn send_cancel(&self, cancel_tokens: Vec<tokio_postgres::CancelToken>) -> Result<()> {
        if cancel_tokens.is_empty() {
            return Ok(());
        }
        let tls = crate::connect::tls::build(&self.settings.connection, "cancel")?;
        let mut failure = None;
        for token in cancel_tokens {
            if let Err(error) = token.cancel_query(tls.connector.clone()).await {
                failure = Some(Error::ProtocolFailed {
                    detail: format!("the cancel request failed: {error}"),
                });
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn track(&self, session: &Session) -> Tracked<'_> {
        let key = CALL_ID.try_with(|id| *id).ok().and_then(|call| {
            let mut running = self.running.lock().ok()?;
            running.next_seq += 1;
            let seq = running.next_seq;
            running
                .cancel_tokens
                .entry(call)
                .or_default()
                .push((seq, session.cancel.clone()));
            Some((call, seq))
        });
        Tracked { engine: self, key }
    }

    pub async fn open_cursors(&self) -> Vec<CursorSummary> {
        let now = Instant::now();
        let mut out = Vec::new();
        {
            let primary = self.primary.lock().await;
            out.extend(summaries(&primary.lane, now));
        }
        if let Some(lane) = self.secondary.lock().await.as_ref() {
            out.extend(summaries(lane, now));
        }
        for lane in self.pinned.lock().await.iter() {
            out.extend(summaries(lane, now));
        }
        out
    }

    pub async fn sweep(&self) -> Result<usize> {
        let mut swept = 0;
        let write_open = {
            let mut primary = self.primary.lock().await;
            swept += sweep_expired(&mut primary.lane).await?;
            finish_if_idle(&mut primary.lane).await?;
            self.expire_write_handle(&mut primary).await?;
            primary.write_handle.is_some()
        };
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            swept += sweep_expired(lane).await?;
            finish_if_idle(lane).await?;
        }
        if !write_open {
            self.drop_idle_secondary().await;
        }
        {
            let mut pinned = self.pinned.lock().await;
            for lane in pinned.iter_mut() {
                swept += sweep_expired(lane).await?;
                finish_if_idle(lane).await?;
            }
            pinned.retain(|lane| !lane.cursors.is_empty());
        }
        let mut handles = self.pooled_handles.lock().await;
        swept += self.pooled_sweep(&mut handles).await;
        Ok(swept)
    }

    pub async fn close_cursor(&self, id: &str, principal: &str) -> Result<()> {
        {
            let mut primary = self.primary.lock().await;
            if primary.lane.cursors.contains_key(id) {
                check_cursor_owner(&primary.lane, id, principal)?;
                close_cursor_now(&mut primary.lane, id).await?;
                return finish_if_idle(&mut primary.lane).await;
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut()
            && lane.cursors.contains_key(id)
        {
            check_cursor_owner(lane, id, principal)?;
            close_cursor_now(lane, id).await?;
            return finish_if_idle(lane).await;
        }
        let mut pinned = self.pinned.lock().await;
        if let Some(position) = pinned.iter().position(|lane| lane.cursors.contains_key(id)) {
            if let Some(lane) = pinned.get(position) {
                check_cursor_owner(lane, id, principal)?;
            }
            let mut lane = pinned.remove(position);
            close_cursor_now(&mut lane, id).await?;
            let outcome = finish_if_idle(&mut lane).await;
            if !lane.cursors.is_empty() {
                pinned.push(lane);
            }
            return outcome;
        }
        Err(Error::HandleState {
            handle: id.to_owned(),
            state: "unknown or expired".to_owned(),
        })
    }

    fn transaction_prefix(&self) -> Option<&str> {
        self.transaction_prefix.as_deref()
    }

    async fn ensure_secondary(&self, slot: &mut Option<Lane>) -> Result<()> {
        let alive = match slot {
            Some(lane) => lane.conn.is_alive().await,
            None => false,
        };
        if alive {
            return Ok(());
        }
        tracing::debug!("opening the second connection");
        let session = self.connector.connect().await?;
        *slot = Some(Lane::new(session));
        Ok(())
    }

    async fn drop_idle_secondary(&self) {
        let mut slot = self.secondary.lock().await;
        if slot.as_ref().is_some_and(|lane| lane.cursors.is_empty()) {
            tracing::debug!("closing the second connection");
            *slot = None;
        }
    }

    async fn read_hold(&self) -> Result<LaneHold<'_>> {
        if self.pool.is_some() {
            return Ok(LaneHold::Pooled(Box::new(self.checkout().await?)));
        }
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        if primary.write_handle.is_none() {
            return Ok(LaneHold::Primary(primary));
        }
        drop(primary);
        let mut slot = self.secondary.lock().await;
        self.ensure_secondary(&mut slot).await?;
        Ok(LaneHold::Secondary(slot))
    }

    async fn release_hold(&self, hold: LaneHold<'_>) {
        if let LaneHold::Pooled(lane) = hold
            && !lane.cursors.is_empty()
        {
            self.pinned.lock().await.push(*lane);
        }
    }

    pub async fn run_read(&self, sql: &str, caps: Caps) -> Result<ResultSet> {
        let expiry = self.settings.limits.cursor_expiry.value;
        let mut hold = self.read_hold().await?;
        let lane = hold.lane()?;
        let _tracked = self.track(lane.conn.session());
        let result = read_on_lane(lane, self.transaction_prefix(), sql, None, caps, expiry).await;
        self.release_hold(hold).await;
        result
    }

    pub async fn run_read_paged(
        &self,
        sql: &str,
        page_with_cursor: bool,
        caps: Caps,
        principal: &str,
    ) -> Result<ResultSet> {
        let expiry = self.settings.limits.cursor_expiry.value;
        let owner = page_with_cursor.then_some(principal);
        let mut hold = self.read_hold().await?;
        let lane = hold.lane()?;
        let _tracked = self.track(lane.conn.session());
        let result = read_on_lane(lane, self.transaction_prefix(), sql, owner, caps, expiry).await;
        self.release_hold(hold).await;
        result
    }

    pub async fn fetch(&self, id: &str, caps: Caps, principal: &str) -> Result<ResultSet> {
        let expiry = self.settings.limits.cursor_expiry.value;
        {
            let mut primary = self.primary.lock().await;
            sweep_expired(&mut primary.lane).await?;
            if primary.lane.cursors.contains_key(id) {
                check_cursor_owner(&primary.lane, id, principal)?;
                let _tracked = self.track(primary.lane.conn.session());
                return fetch_on_lane(&mut primary.lane, id, caps, expiry).await;
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            sweep_expired(lane).await?;
            if lane.cursors.contains_key(id) {
                check_cursor_owner(lane, id, principal)?;
                let _tracked = self.track(lane.conn.session());
                return fetch_on_lane(lane, id, caps, expiry).await;
            }
        }
        let mut pinned = self.pinned.lock().await;
        for lane in pinned.iter_mut() {
            sweep_expired(lane).await?;
        }
        if let Some(position) = pinned.iter().position(|lane| lane.cursors.contains_key(id)) {
            if let Some(lane) = pinned.get(position) {
                check_cursor_owner(lane, id, principal)?;
            }
            let mut lane = pinned.remove(position);
            let _tracked = self.track(lane.conn.session());
            let result = fetch_on_lane(&mut lane, id, caps, expiry).await;
            if !lane.cursors.is_empty() {
                pinned.push(lane);
            }
            return result;
        }
        pinned.retain(|lane| !lane.cursors.is_empty());
        Err(Error::HandleState {
            handle: id.to_owned(),
            state: "unknown or expired".to_owned(),
        })
    }

    pub fn catalog_rows<'a>(
        &'a self,
        sql: &'a str,
        params: &'a [&'a dyn CatalogParam],
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<tokio_postgres::Row>>> {
        Box::pin(async move {
            let mut hold = self.read_hold().await?;
            let lane = hold.lane()?;
            let _tracked = self.track(lane.conn.session());
            let result = catalog_on_lane(lane, self.transaction_prefix(), sql, params).await;
            self.release_hold(hold).await;
            result
        })
    }

    async fn ensure_alive(&self, primary: &mut Primary) -> Result<()> {
        if primary.lane.conn.is_alive().await {
            return Ok(());
        }
        tracing::warn!("the database connection was lost; reconnecting once");
        if let Some(handle) = primary.write_handle.take() {
            primary.closed.insert(handle.id, HandleState::Lost);
        }
        let fresh = self.connector.connect().await?;
        primary.lane = Lane::new(fresh);
        Ok(())
    }

    async fn expire_write_handle(&self, primary: &mut Primary) -> Result<()> {
        let expired = primary
            .write_handle
            .as_ref()
            .is_some_and(|handle| handle.expires_at <= Instant::now());
        if !expired {
            return Ok(());
        }
        if let Some(handle) = primary.write_handle.take() {
            tracing::info!(handle = %handle.id, "the transaction handle expired; rolling back");
            let outcome = primary.lane.conn.client().batch_execute("ROLLBACK").await;
            if let Err(error) = outcome
                && !primary.lane.conn.client().is_closed()
            {
                return Err(describe_sqlstate(&error));
            }
            primary.closed.insert(handle.id, HandleState::Expired);
        }
        Ok(())
    }

    fn check_handle<'a>(
        primary: &'a mut Primary,
        id: &str,
        principal: &str,
    ) -> Result<&'a mut WriteHandle> {
        match primary.write_handle.as_mut() {
            Some(handle) if handle.id == id => {
                if handle.principal != principal {
                    return Err(Error::HandleState {
                        handle: id.to_owned(),
                        state: "owned by another principal".to_owned(),
                    });
                }
                Ok(handle)
            }
            _ => {
                let state = primary
                    .closed
                    .get(id)
                    .map_or("unknown", |state| state.as_str());
                Err(Error::HandleState {
                    handle: id.to_owned(),
                    state: state.to_owned(),
                })
            }
        }
    }

    fn describe_handle(handle: &WriteHandle) -> HandleInfo {
        HandleInfo {
            id: handle.id.clone(),
            state: HandleState::Open,
            principal: handle.principal.clone(),
            savepoints: handle.savepoints.clone(),
            statements: handle.statements,
            expires_in_seconds: handle
                .expires_at
                .saturating_duration_since(Instant::now())
                .as_secs(),
        }
    }

    pub async fn begin_transaction(&self, principal: &str) -> Result<HandleInfo> {
        if self.pool.is_some() {
            return self.pooled_begin(principal).await;
        }
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        if let Some(handle) = &primary.write_handle {
            return Err(Error::HandleState {
                handle: handle.id.clone(),
                state: "open; commit or roll it back before opening another".to_owned(),
            });
        }
        rollback_all(&mut primary.lane).await?;
        let _tracked = self.track(primary.lane.conn.session());
        primary
            .lane
            .conn
            .client()
            .batch_execute(&begin_statement(self.transaction_prefix()))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let handle = self.new_write_handle(principal);
        let info = Self::describe_handle(&handle);
        primary.write_handle = Some(handle);
        Ok(info)
    }

    fn new_write_handle(&self, principal: &str) -> WriteHandle {
        WriteHandle {
            id: new_handle_id(),
            principal: principal.to_owned(),
            savepoints: Vec::new(),
            statements: 0,
            expires_at: Instant::now() + self.settings.limits.handle_expiry.value,
        }
    }

    pub async fn transaction_status(&self, id: &str) -> Result<HandleInfo> {
        if self.pool.is_some() {
            return self.pooled_status(id).await;
        }
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        if let Some(handle) = primary
            .write_handle
            .as_ref()
            .filter(|handle| handle.id == id)
        {
            return Ok(Self::describe_handle(handle));
        }
        closed_handle_info(&primary.closed, id)
    }

    pub async fn open_transaction_count(&self) -> usize {
        if self.pool.is_some() {
            return self.pooled_handles.lock().await.open.len();
        }
        usize::from(self.primary.lock().await.write_handle.is_some())
    }

    pub async fn commit(&self, id: &str, principal: &str) -> Result<HandleInfo> {
        self.finish_transaction(id, principal, "COMMIT", HandleState::Committed)
            .await
    }

    pub async fn rollback(&self, id: &str, principal: &str) -> Result<HandleInfo> {
        self.finish_transaction(id, principal, "ROLLBACK", HandleState::RolledBack)
            .await
    }

    async fn finish_transaction(
        &self,
        id: &str,
        principal: &str,
        statement: &str,
        state: HandleState,
    ) -> Result<HandleInfo> {
        if self.pool.is_some() {
            return self.pooled_finish(id, principal, statement, state).await;
        }
        let outcome = {
            let mut primary = self.primary.lock().await;
            self.expire_write_handle(&mut primary).await?;
            Self::check_handle(&mut primary, id, principal)?;
            let _tracked = self.track(primary.lane.conn.session());
            if primary.lane.conn.client().is_closed() {
                if let Some(handle) = primary.write_handle.take() {
                    primary.closed.insert(handle.id, HandleState::Lost);
                }
                return Err(Error::HandleState {
                    handle: id.to_owned(),
                    state: "lost".to_owned(),
                });
            }
            let outcome = primary.lane.conn.client().batch_execute(statement).await;
            let Some(handle) = primary.write_handle.take() else {
                return Err(Error::HandleState {
                    handle: id.to_owned(),
                    state: "unknown".to_owned(),
                });
            };
            let mut info = Self::describe_handle(&handle);
            match outcome {
                Ok(()) => {
                    info.state = state;
                    info.expires_in_seconds = 0;
                    primary.closed.insert(handle.id, state);
                    Ok(info)
                }
                Err(error) => {
                    let final_state = if primary.lane.conn.client().is_closed() {
                        HandleState::Lost
                    } else {
                        HandleState::RolledBack
                    };
                    primary.closed.insert(handle.id, final_state);
                    Err(describe_sqlstate(&error))
                }
            }
        };
        self.drop_idle_secondary().await;
        outcome
    }

    pub async fn savepoint(&self, id: &str, principal: &str, name: &str) -> Result<HandleInfo> {
        let quoted = crate::render::quote_ident(name);
        let statement = format!("SAVEPOINT {quoted}");
        if self.pool.is_some() {
            return self
                .pooled_on_handle(
                    id,
                    principal,
                    |handle| {
                        handle.savepoints.push(name.to_owned());
                    },
                    &statement,
                )
                .await;
        }
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        Self::check_handle(&mut primary, id, principal)?;
        let _tracked = self.track(primary.lane.conn.session());
        primary
            .lane
            .conn
            .client()
            .batch_execute(&statement)
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let expiry = self.settings.limits.handle_expiry.value;
        let handle = Self::check_handle(&mut primary, id, principal)?;
        handle.savepoints.push(name.to_owned());
        handle.expires_at = Instant::now() + expiry;
        Ok(Self::describe_handle(handle))
    }

    pub async fn rollback_to(&self, id: &str, principal: &str, name: &str) -> Result<HandleInfo> {
        let quoted = crate::render::quote_ident(name);
        let statement = format!("ROLLBACK TO SAVEPOINT {quoted}");
        if self.pool.is_some() {
            self.pooled_check_savepoint(id, principal, name).await?;
            return self
                .pooled_on_handle(
                    id,
                    principal,
                    |handle| truncate_savepoints(handle, name),
                    &statement,
                )
                .await;
        }
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        {
            let handle = Self::check_handle(&mut primary, id, principal)?;
            check_savepoint(handle, id, name)?;
        }
        let _tracked = self.track(primary.lane.conn.session());
        primary
            .lane
            .conn
            .client()
            .batch_execute(&statement)
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let expiry = self.settings.limits.handle_expiry.value;
        let handle = Self::check_handle(&mut primary, id, principal)?;
        truncate_savepoints(handle, name);
        handle.expires_at = Instant::now() + expiry;
        Ok(Self::describe_handle(handle))
    }

    pub async fn run_write(
        &self,
        sql: &str,
        caps: Caps,
        principal: &str,
        handle: Option<&str>,
        outside_transaction: bool,
    ) -> Result<ResultSet> {
        self.run_write_with(sql, caps, principal, handle, outside_transaction, None)
            .await
    }

    pub async fn run_write_with(
        &self,
        sql: &str,
        caps: Caps,
        principal: &str,
        handle: Option<&str>,
        outside_transaction: bool,
        timeout: Option<Duration>,
    ) -> Result<ResultSet> {
        if let Some(id) = handle {
            if outside_transaction {
                return Err(Error::StatementRefused {
                    rule: "this statement cannot run inside a transaction block; call it without a transaction handle".to_owned(),
                    mode: self.settings.mode.value.to_string(),
                });
            }
            if self.pool.is_some() {
                return self
                    .pooled_run_in_handle(id, principal, sql, caps, timeout)
                    .await;
            }
            let mut primary = self.primary.lock().await;
            self.expire_write_handle(&mut primary).await?;
            Self::check_handle(&mut primary, id, principal)?;
            let _tracked = self.track(primary.lane.conn.session());
            if primary.lane.conn.client().is_closed() {
                if let Some(handle) = primary.write_handle.take() {
                    primary.closed.insert(handle.id, HandleState::Lost);
                }
                return Err(Error::HandleState {
                    handle: id.to_owned(),
                    state: "lost".to_owned(),
                });
            }
            let result = timed(primary.lane.conn.client(), timeout, true, || {
                guarded_statement(primary.lane.conn.client(), sql, caps)
            })
            .await;
            let expiry = self.settings.limits.handle_expiry.value;
            if primary.lane.conn.client().is_closed() {
                if let Some(handle) = primary.write_handle.take() {
                    primary.closed.insert(handle.id, HandleState::Lost);
                }
            } else if let Ok(handle) = Self::check_handle(&mut primary, id, principal) {
                handle.statements += 1;
                handle.expires_at = Instant::now() + expiry;
            }
            return result;
        }
        if self.pool.is_some() {
            let lane = self.checkout().await?;
            let _tracked = self.track(lane.conn.session());
            return autocommit(
                lane.conn.client(),
                self.transaction_prefix(),
                timeout,
                sql,
                caps,
            )
            .await;
        }
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        if let Some(open) = &primary.write_handle {
            return Err(Error::HandleState {
                handle: open.id.clone(),
                state: "open; pass it as `transaction`, or commit or roll it back first".to_owned(),
            });
        }
        sweep_expired(&mut primary.lane).await?;
        finish_if_idle(&mut primary.lane).await?;
        if !primary.lane.in_read_transaction {
            let _tracked = self.track(primary.lane.conn.session());
            return autocommit(
                primary.lane.conn.client(),
                self.transaction_prefix(),
                timeout,
                sql,
                caps,
            )
            .await;
        }
        drop(primary);
        let mut slot = self.secondary.lock().await;
        self.ensure_secondary(&mut slot).await?;
        let lane = slot.as_mut().ok_or_else(|| Error::ProtocolFailed {
            detail: "the second connection is missing".to_owned(),
        })?;
        sweep_expired(lane).await?;
        finish_if_idle(lane).await?;
        if lane.in_read_transaction {
            return Err(Error::HandleState {
                handle: "cursors".to_owned(),
                state: "open on both connections; close a cursor or let it expire before writing"
                    .to_owned(),
            });
        }
        let _tracked = self.track(lane.conn.session());
        autocommit(
            lane.conn.client(),
            self.transaction_prefix(),
            timeout,
            sql,
            caps,
        )
        .await
    }

    pub async fn run_unparsed(
        &self,
        sql: &str,
        caps: Caps,
        principal: &str,
        handle: Option<&str>,
    ) -> Result<ResultSet> {
        {
            let mut hold = self.read_hold().await?;
            let lane = hold.lane()?;
            let _tracked = self.track(lane.conn.session());
            let statement = lane
                .conn
                .client()
                .prepare(sql)
                .await
                .map_err(|error| describe_sqlstate(&error))?;
            drop(statement);
        }
        self.run_write_with(sql, caps, principal, handle, false, None)
            .await
    }

    pub async fn primary_backend_pid(&self) -> Result<i32> {
        let primary = self.primary.lock().await;
        let rows = query_rows(
            primary.lane.conn.client(),
            "SELECT pg_catalog.pg_backend_pid()",
            &[],
        )
        .await?;
        rows.first()
            .map(|row| row.try_get::<_, i32>(0))
            .transpose()
            .map_err(|error| describe_sqlstate(&error))?
            .ok_or_else(|| Error::ProtocolFailed {
                detail: "the backend pid could not be read".to_owned(),
            })
    }

    pub async fn catalog_rows_off_primary(
        &self,
        sql: &str,
        params: &[&dyn CatalogParam],
    ) -> Result<Vec<tokio_postgres::Row>> {
        if self.pool.is_some() {
            let mut lane = self.checkout().await?;
            let _tracked = self.track(lane.conn.session());
            return catalog_on_lane(&mut lane, self.transaction_prefix(), sql, params).await;
        }
        let mut slot = self.secondary.lock().await;
        self.ensure_secondary(&mut slot).await?;
        let lane = slot.as_mut().ok_or_else(|| Error::ProtocolFailed {
            detail: "the second connection is missing".to_owned(),
        })?;
        let _tracked = self.track(lane.conn.session());
        catalog_on_lane(lane, self.transaction_prefix(), sql, params).await
    }

    pub async fn copy_in(
        &self,
        sql: &str,
        data: &[u8],
        principal: &str,
        handle: Option<&str>,
    ) -> Result<u64> {
        if self.pool.is_some() {
            return self.pooled_copy_in(sql, data, principal, handle).await;
        }
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        match (handle, &primary.write_handle) {
            (Some(id), _) => {
                Self::check_handle(&mut primary, id, principal)?;
            }
            (None, Some(open)) => {
                return Err(Error::HandleState {
                    handle: open.id.clone(),
                    state: "open; pass it as `transaction`, or commit or roll it back first"
                        .to_owned(),
                });
            }
            (None, None) => {
                sweep_expired(&mut primary.lane).await?;
                finish_if_idle(&mut primary.lane).await?;
                if primary.lane.in_read_transaction {
                    return Err(Error::HandleState {
                        handle: "cursors".to_owned(),
                        state: "open; close a cursor or let it expire before a COPY".to_owned(),
                    });
                }
            }
        }
        let _tracked = self.track(primary.lane.conn.session());
        let rows = copy_in_on(
            primary.lane.conn.client(),
            self.transaction_prefix(),
            handle.is_some(),
            sql,
            data,
        )
        .await?;
        if let Some(id) = handle
            && let Ok(open) = Self::check_handle(&mut primary, id, principal)
        {
            open.statements += 1;
            open.expires_at = Instant::now() + self.settings.limits.handle_expiry.value;
        }
        Ok(rows)
    }

    pub async fn copy_out(&self, sql: &str, byte_cap: usize) -> Result<(Vec<u8>, bool)> {
        let mut hold = self.read_hold().await?;
        let lane = hold.lane()?;
        let _tracked = self.track(lane.conn.session());
        let result = copy_out_on(lane, self.transaction_prefix(), sql, byte_cap).await;
        self.release_hold(hold).await;
        result
    }

    pub async fn run_and_rollback(&self, sql: &str, caps: Caps) -> Result<ResultSet> {
        if self.pool.is_some() {
            let lane = self.checkout().await?;
            let _tracked = self.track(lane.conn.session());
            return run_then_rollback(lane.conn.client(), self.transaction_prefix(), sql, caps)
                .await;
        }
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        if let Some(open) = &primary.write_handle {
            return Err(Error::HandleState {
                handle: open.id.clone(),
                state: "open; commit or roll it back before an EXPLAIN ANALYZE of a write"
                    .to_owned(),
            });
        }
        sweep_expired(&mut primary.lane).await?;
        finish_if_idle(&mut primary.lane).await?;
        if !primary.lane.in_read_transaction {
            let _tracked = self.track(primary.lane.conn.session());
            return run_then_rollback(
                primary.lane.conn.client(),
                self.transaction_prefix(),
                sql,
                caps,
            )
            .await;
        }
        drop(primary);
        let mut slot = self.secondary.lock().await;
        self.ensure_secondary(&mut slot).await?;
        let lane = slot.as_mut().ok_or_else(|| Error::ProtocolFailed {
            detail: "the second connection is missing".to_owned(),
        })?;
        sweep_expired(lane).await?;
        finish_if_idle(lane).await?;
        if lane.in_read_transaction {
            return Err(Error::HandleState {
                handle: "cursors".to_owned(),
                state: "open on both connections; close a cursor or let it expire first".to_owned(),
            });
        }
        let _tracked = self.track(lane.conn.session());
        run_then_rollback(lane.conn.client(), self.transaction_prefix(), sql, caps).await
    }

    pub async fn release_everything(&self) -> Result<()> {
        {
            let mut primary = self.primary.lock().await;
            if let Some(handle) = primary.write_handle.take() {
                if !primary.lane.conn.client().is_closed() {
                    let _ = primary.lane.conn.client().batch_execute("ROLLBACK").await;
                }
                primary.closed.insert(handle.id, HandleState::RolledBack);
            }
            if primary.lane.conn.client().is_closed() {
                primary.lane.forget_state();
            } else {
                rollback_all(&mut primary.lane).await?;
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            if lane.conn.client().is_closed() {
                lane.forget_state();
            } else {
                rollback_all(lane).await?;
            }
        }
        let mut pinned = self.pinned.lock().await;
        for lane in pinned.iter_mut() {
            if lane.conn.client().is_closed() {
                lane.forget_state();
            } else {
                let _ = rollback_all(lane).await;
            }
        }
        pinned.clear();
        {
            let mut handles = self.pooled_handles.lock().await;
            let open: Vec<(String, PooledHandle)> = handles.open.drain().collect();
            for (id, held) in open {
                if !held.lane.conn.client().is_closed() {
                    let _ = held.lane.conn.client().batch_execute("ROLLBACK").await;
                }
                handles.closed.insert(id, HandleState::RolledBack);
            }
        }
        if let Some(pool) = &self.pool {
            pool.close();
        }
        Ok(())
    }

    async fn pooled_sweep(&self, handles: &mut PooledHandles) -> usize {
        let now = Instant::now();
        let expired: Vec<String> = handles
            .open
            .iter()
            .filter(|(_, held)| held.handle.expires_at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        let count = expired.len();
        for id in expired {
            if let Some(held) = handles.open.remove(&id) {
                tracing::info!(handle = %id, "the transaction handle expired; rolling back");
                if !held.lane.conn.client().is_closed() {
                    let _ = held.lane.conn.client().batch_execute("ROLLBACK").await;
                }
                handles.closed.insert(id, HandleState::Expired);
            }
        }
        count
    }

    async fn pooled_begin(&self, principal: &str) -> Result<HandleInfo> {
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        if let Some(open) = handles
            .open
            .values()
            .find(|held| held.handle.principal == principal)
        {
            return Err(Error::HandleState {
                handle: open.handle.id.clone(),
                state: "open; commit or roll it back before opening another".to_owned(),
            });
        }
        let lane = self.checkout().await?;
        let _tracked = self.track(lane.conn.session());
        lane.conn
            .client()
            .batch_execute(&begin_statement(self.transaction_prefix()))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let handle = self.new_write_handle(principal);
        let info = Self::describe_handle(&handle);
        handles
            .open
            .insert(handle.id.clone(), PooledHandle { lane, handle });
        Ok(info)
    }

    async fn pooled_status(&self, id: &str) -> Result<HandleInfo> {
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        if let Some(held) = handles.open.get(id) {
            return Ok(Self::describe_handle(&held.handle));
        }
        closed_handle_info(&handles.closed, id)
    }

    fn pooled_check_handle<'a>(
        handles: &'a mut PooledHandles,
        id: &str,
        principal: &str,
    ) -> Result<&'a mut PooledHandle> {
        match handles.open.get_mut(id) {
            Some(held) if held.handle.principal == principal => Ok(held),
            Some(_) => Err(Error::HandleState {
                handle: id.to_owned(),
                state: "owned by another principal".to_owned(),
            }),
            None => Err(Error::HandleState {
                handle: id.to_owned(),
                state: handles
                    .closed
                    .get(id)
                    .map_or("unknown", |state| state.as_str())
                    .to_owned(),
            }),
        }
    }

    fn pooled_lose(handles: &mut PooledHandles, id: &str) {
        if handles.open.remove(id).is_some() {
            handles.closed.insert(id.to_owned(), HandleState::Lost);
        }
    }

    async fn pooled_finish(
        &self,
        id: &str,
        principal: &str,
        statement: &str,
        state: HandleState,
    ) -> Result<HandleInfo> {
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        Self::pooled_check_handle(&mut handles, id, principal)?;
        let Some(held) = handles.open.remove(id) else {
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "unknown".to_owned(),
            });
        };
        if held.lane.conn.client().is_closed() {
            handles.closed.insert(id.to_owned(), HandleState::Lost);
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "lost".to_owned(),
            });
        }
        let _tracked = self.track(held.lane.conn.session());
        let outcome = held.lane.conn.client().batch_execute(statement).await;
        let mut info = Self::describe_handle(&held.handle);
        match outcome {
            Ok(()) => {
                info.state = state;
                info.expires_in_seconds = 0;
                handles.closed.insert(id.to_owned(), state);
                Ok(info)
            }
            Err(error) => {
                let final_state = if held.lane.conn.client().is_closed() {
                    HandleState::Lost
                } else {
                    HandleState::RolledBack
                };
                handles.closed.insert(id.to_owned(), final_state);
                Err(describe_sqlstate(&error))
            }
        }
    }

    async fn pooled_check_savepoint(&self, id: &str, principal: &str, name: &str) -> Result<()> {
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        let held = Self::pooled_check_handle(&mut handles, id, principal)?;
        check_savepoint(&held.handle, id, name)
    }

    async fn pooled_on_handle<F>(
        &self,
        id: &str,
        principal: &str,
        update: F,
        statement: &str,
    ) -> Result<HandleInfo>
    where
        F: FnOnce(&mut WriteHandle),
    {
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        let held = Self::pooled_check_handle(&mut handles, id, principal)?;
        if held.lane.conn.client().is_closed() {
            Self::pooled_lose(&mut handles, id);
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "lost".to_owned(),
            });
        }
        let _tracked = self.track(held.lane.conn.session());
        held.lane
            .conn
            .client()
            .batch_execute(statement)
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        update(&mut held.handle);
        held.handle.expires_at = Instant::now() + self.settings.limits.handle_expiry.value;
        Ok(Self::describe_handle(&held.handle))
    }

    async fn pooled_run_in_handle(
        &self,
        id: &str,
        principal: &str,
        sql: &str,
        caps: Caps,
        timeout: Option<Duration>,
    ) -> Result<ResultSet> {
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        let held = Self::pooled_check_handle(&mut handles, id, principal)?;
        if held.lane.conn.client().is_closed() {
            Self::pooled_lose(&mut handles, id);
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "lost".to_owned(),
            });
        }
        let _tracked = self.track(held.lane.conn.session());
        let client = held.lane.conn.client();
        let result = timed(client, timeout, true, || {
            guarded_statement(client, sql, caps)
        })
        .await;
        if held.lane.conn.client().is_closed() {
            Self::pooled_lose(&mut handles, id);
        } else {
            held.handle.statements += 1;
            held.handle.expires_at = Instant::now() + self.settings.limits.handle_expiry.value;
        }
        result
    }

    async fn pooled_copy_in(
        &self,
        sql: &str,
        data: &[u8],
        principal: &str,
        handle: Option<&str>,
    ) -> Result<u64> {
        let Some(id) = handle else {
            let lane = self.checkout().await?;
            let _tracked = self.track(lane.conn.session());
            return copy_in_on(
                lane.conn.client(),
                self.transaction_prefix(),
                false,
                sql,
                data,
            )
            .await;
        };
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        let held = Self::pooled_check_handle(&mut handles, id, principal)?;
        if held.lane.conn.client().is_closed() {
            Self::pooled_lose(&mut handles, id);
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "lost".to_owned(),
            });
        }
        let _tracked = self.track(held.lane.conn.session());
        let rows = copy_in_on(
            held.lane.conn.client(),
            self.transaction_prefix(),
            true,
            sql,
            data,
        )
        .await?;
        held.handle.statements += 1;
        held.handle.expires_at = Instant::now() + self.settings.limits.handle_expiry.value;
        Ok(rows)
    }
}

enum LaneHold<'a> {
    Pooled(Box<Lane>),
    Primary(tokio::sync::MutexGuard<'a, Primary>),
    Secondary(tokio::sync::MutexGuard<'a, Option<Lane>>),
}

impl LaneHold<'_> {
    fn lane(&mut self) -> Result<&mut Lane> {
        match self {
            Self::Pooled(lane) => Ok(lane),
            Self::Primary(primary) => Ok(&mut primary.lane),
            Self::Secondary(slot) => slot.as_mut().ok_or_else(|| Error::ProtocolFailed {
                detail: "the second connection is missing".to_owned(),
            }),
        }
    }
}

fn begin_statement(transaction_prefix: Option<&str>) -> String {
    match transaction_prefix {
        Some(transaction_prefix) => format!("BEGIN; {transaction_prefix}"),
        None => "BEGIN".to_owned(),
    }
}

fn closed_handle_info(closed: &BTreeMap<String, HandleState>, id: &str) -> Result<HandleInfo> {
    match closed.get(id) {
        Some(state) => Ok(HandleInfo {
            id: id.to_owned(),
            state: *state,
            principal: String::new(),
            savepoints: Vec::new(),
            statements: 0,
            expires_in_seconds: 0,
        }),
        None => Err(Error::HandleState {
            handle: id.to_owned(),
            state: "unknown".to_owned(),
        }),
    }
}

fn check_savepoint(handle: &WriteHandle, id: &str, name: &str) -> Result<()> {
    if handle.savepoints.iter().any(|known| known == name) {
        return Ok(());
    }
    Err(Error::HandleState {
        handle: id.to_owned(),
        state: format!("holding no savepoint named `{name}`"),
    })
}

fn truncate_savepoints(handle: &mut WriteHandle, name: &str) {
    if let Some(position) = handle.savepoints.iter().position(|known| known == name) {
        handle.savepoints.truncate(position + 1);
    }
}

fn check_cursor_owner(lane: &Lane, id: &str, principal: &str) -> Result<()> {
    match lane.cursors.get(id) {
        Some(cursor) if cursor.principal == principal => Ok(()),
        Some(_) => Err(Error::HandleState {
            handle: id.to_owned(),
            state: "owned by another principal".to_owned(),
        }),
        None => Err(Error::HandleState {
            handle: id.to_owned(),
            state: "unknown or expired".to_owned(),
        }),
    }
}

async fn catalog_on_lane(
    lane: &mut Lane,
    transaction_prefix: Option<&str>,
    sql: &str,
    params: &[&dyn CatalogParam],
) -> Result<Vec<tokio_postgres::Row>> {
    if transaction_prefix.is_none() {
        return query_rows(lane.conn.client(), sql, params).await;
    }
    sweep_expired(lane).await?;
    begin_read(lane, transaction_prefix).await?;
    let outcome = query_rows(lane.conn.client(), sql, params).await;
    if lane.conn.client().is_closed() {
        lane.forget_state();
    } else if outcome.is_ok() {
        finish_if_idle(lane).await?;
    } else {
        let _ = finish_if_idle(lane).await;
    }
    outcome
}

async fn autocommit(
    client: &tokio_postgres::Client,
    transaction_prefix: Option<&str>,
    timeout: Option<Duration>,
    sql: &str,
    caps: Caps,
) -> Result<ResultSet> {
    let Some(transaction_prefix) = transaction_prefix else {
        return timed(client, timeout, false, || {
            autocommit_statement(client, sql, caps)
        })
        .await;
    };
    client
        .batch_execute(&format!("BEGIN; {transaction_prefix}"))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let result = timed(client, timeout, true, || {
        autocommit_statement(client, sql, caps)
    })
    .await;
    settle_wrapper(client, result.is_ok()).await?;
    result
}

async fn settle_wrapper(client: &tokio_postgres::Client, commit: bool) -> Result<()> {
    if client.is_closed() {
        return Ok(());
    }
    let outcome = client
        .batch_execute(if commit { "COMMIT" } else { "ROLLBACK" })
        .await;
    match outcome {
        Ok(()) => Ok(()),
        Err(error) if commit => Err(describe_sqlstate(&error)),
        Err(_) => Ok(()),
    }
}

async fn copy_in_on(
    client: &tokio_postgres::Client,
    transaction_prefix: Option<&str>,
    inside_handle: bool,
    sql: &str,
    data: &[u8],
) -> Result<u64> {
    let wrap = transaction_prefix.filter(|_| !inside_handle);
    if let Some(transaction_prefix) = wrap {
        client
            .batch_execute(&format!("BEGIN; {transaction_prefix}"))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
    }
    let result = async {
        let sink = client
            .copy_in(sql)
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let mut sink = std::pin::pin!(sink);
        sink.send(bytes::Bytes::copy_from_slice(data))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        sink.finish()
            .await
            .map_err(|error| describe_sqlstate(&error))
    }
    .await;
    if wrap.is_some() {
        settle_wrapper(client, result.is_ok()).await?;
    }
    result
}

async fn copy_out_on(
    lane: &mut Lane,
    transaction_prefix: Option<&str>,
    sql: &str,
    byte_cap: usize,
) -> Result<(Vec<u8>, bool)> {
    sweep_expired(lane).await?;
    begin_read(lane, transaction_prefix).await?;
    let session = lane.conn.session();
    let outcome = async {
        let stream = session
            .client
            .copy_out(sql)
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let mut stream = std::pin::pin!(stream);
        let mut out = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|error| describe_sqlstate(&error))?
        {
            if out.len() + chunk.len() > byte_cap {
                let room = byte_cap.saturating_sub(out.len());
                out.extend_from_slice(chunk.get(..room).unwrap_or_default());
                truncated = true;
                break;
            }
            out.extend_from_slice(&chunk);
        }
        if truncated {
            let _ = session.cancel_running_statement().await;
            while stream.try_next().await.ok().flatten().is_some() {}
        }
        Ok((out, truncated))
    }
    .await;
    if lane.conn.client().is_closed() {
        lane.forget_state();
    } else {
        let _ = finish_if_idle(lane).await;
    }
    outcome
}

fn summaries(lane: &Lane, now: Instant) -> Vec<CursorSummary> {
    lane.cursors
        .iter()
        .map(|(id, cursor)| CursorSummary {
            id: id.clone(),
            expires_in_seconds: cursor.expires_at.saturating_duration_since(now).as_secs(),
        })
        .collect()
}

async fn query_rows(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[&dyn CatalogParam],
) -> Result<Vec<tokio_postgres::Row>> {
    let typed = params
        .iter()
        .map(|param| (*param as &(dyn ToSql + Sync), param.pg_type()));
    let stream = client
        .query_typed_raw(sql, typed)
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let mut stream = std::pin::pin!(stream);
    let mut rows = Vec::new();
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|error| describe_sqlstate(&error))?
    {
        rows.push(row);
    }
    Ok(rows)
}

async fn read_on_lane(
    lane: &mut Lane,
    transaction_prefix: Option<&str>,
    sql: &str,
    owner: Option<&str>,
    caps: Caps,
    expiry: Duration,
) -> Result<ResultSet> {
    sweep_expired(lane).await?;
    begin_read(lane, transaction_prefix).await?;
    let outcome = match owner {
        Some(principal) if lane.cursors.len() < CURSOR_CAP => {
            read_through_cursor(lane, sql, principal, caps, expiry).await
        }
        _ => guarded_statement(lane.conn.client(), sql, caps).await,
    };
    match outcome {
        Ok(result) => {
            finish_if_idle(lane).await?;
            Ok(result)
        }
        Err(error) => {
            if lane.conn.client().is_closed() {
                lane.forget_state();
            } else {
                let _ = finish_if_idle(lane).await;
            }
            Err(error)
        }
    }
}

async fn fetch_on_lane(
    lane: &mut Lane,
    id: &str,
    caps: Caps,
    expiry: Duration,
) -> Result<ResultSet> {
    let Some(cursor) = lane.cursors.get_mut(id) else {
        return Err(Error::HandleState {
            handle: id.to_owned(),
            state: "unknown or expired".to_owned(),
        });
    };
    let name = cursor.name.clone();
    let columns = cursor.columns.clone();
    let estimate = cursor.estimate;
    let mut pending = std::mem::take(&mut cursor.pending);
    let page = page_from_cursor(lane.conn.client(), &name, &mut pending, columns, caps).await;
    let (collector, more) = match page {
        Ok(page) => page,
        Err(error) => {
            let _ = rollback_all(lane).await;
            return Err(error);
        }
    };
    if more {
        if let Some(cursor) = lane.cursors.get_mut(id) {
            cursor.pending = pending;
            cursor.expires_at = Instant::now() + expiry;
        }
        Ok(collector.finish(Some(id.to_owned()), estimate))
    } else {
        close_cursor_now(lane, id).await?;
        finish_if_idle(lane).await?;
        Ok(collector.finish(None, estimate))
    }
}

async fn read_through_cursor(
    lane: &mut Lane,
    sql: &str,
    principal: &str,
    caps: Caps,
    expiry: Duration,
) -> Result<ResultSet> {
    let client = &lane.conn.client();
    client
        .batch_execute(&format!("SAVEPOINT {SAVEPOINT}"))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let prepared = client.prepare(sql).await;
    let statement = match prepared {
        Ok(statement) => statement,
        Err(error) => {
            rollback_savepoint(client).await;
            return Err(describe_sqlstate(&error));
        }
    };
    let columns: Vec<Column> = statement
        .columns()
        .iter()
        .map(|column| Column {
            name: column.name().to_owned(),
            type_name: column.type_().name().to_owned(),
        })
        .collect();
    lane.next_cursor += 1;
    let name = format!("ownpg_cursor_{}", lane.next_cursor);
    let declared = client
        .batch_execute(&format!(
            "DECLARE {name} NO SCROLL CURSOR WITHOUT HOLD FOR {sql}"
        ))
        .await;
    if let Err(error) = declared {
        rollback_savepoint(client).await;
        return Err(describe_sqlstate(&error));
    }
    let mut pending = VecDeque::new();
    let page = page_from_cursor(
        lane.conn.client(),
        &name,
        &mut pending,
        columns.clone(),
        caps,
    )
    .await;
    let (collector, more) = match page {
        Ok(page) => page,
        Err(error) => {
            rollback_savepoint(client).await;
            return Err(error);
        }
    };
    client
        .batch_execute(&format!("RELEASE SAVEPOINT {SAVEPOINT}"))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    if more {
        let estimate = estimate_rows(client, sql).await;
        let id = new_handle_id();
        lane.cursors.insert(
            id.clone(),
            Cursor {
                name,
                principal: principal.to_owned(),
                columns,
                estimate,
                expires_at: Instant::now() + expiry,
                pending,
            },
        );
        Ok(collector.finish(Some(id), estimate))
    } else {
        client
            .batch_execute(&format!("CLOSE {name}"))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        Ok(collector.finish(None, None))
    }
}

fn new_handle_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

async fn begin_read(lane: &mut Lane, transaction_prefix: Option<&str>) -> Result<()> {
    if lane.in_read_transaction {
        return Ok(());
    }
    let statement = match transaction_prefix {
        Some(transaction_prefix) => {
            format!("BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY; {transaction_prefix}")
        }
        None => "BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY".to_owned(),
    };
    lane.conn
        .client()
        .batch_execute(&statement)
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    lane.in_read_transaction = true;
    Ok(())
}

async fn rollback_savepoint(client: &tokio_postgres::Client) {
    let _ = client
        .batch_execute(&format!(
            "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT}"
        ))
        .await;
}

async fn rollback_all(lane: &mut Lane) -> Result<()> {
    lane.cursors.clear();
    if lane.in_read_transaction {
        lane.in_read_transaction = false;
        lane.conn
            .client()
            .batch_execute("ROLLBACK")
            .await
            .map_err(|error| describe_sqlstate(&error))?;
    }
    Ok(())
}

async fn finish_if_idle(lane: &mut Lane) -> Result<()> {
    if lane.cursors.is_empty() && lane.in_read_transaction {
        rollback_all(lane).await?;
    }
    Ok(())
}

async fn close_cursor_now(lane: &mut Lane, id: &str) -> Result<()> {
    if let Some(cursor) = lane.cursors.remove(id) {
        let closed = lane
            .conn
            .client()
            .batch_execute(&format!("CLOSE {}", cursor.name))
            .await;
        if let Err(error) = closed {
            tracing::debug!(%error, "the cursor was already gone");
        }
    }
    Ok(())
}

async fn sweep_expired(lane: &mut Lane) -> Result<usize> {
    let now = Instant::now();
    let expired: Vec<String> = lane
        .cursors
        .iter()
        .filter(|(_, cursor)| cursor.expires_at <= now)
        .map(|(id, _)| id.clone())
        .collect();
    let count = expired.len();
    for id in expired {
        close_cursor_now(lane, &id).await?;
    }
    Ok(count)
}

async fn fetch_rows(
    client: &tokio_postgres::Client,
    cursor: &str,
    count: usize,
) -> Result<Vec<SimpleQueryMessage>> {
    client
        .simple_query(&format!("FETCH FORWARD {count} FROM {cursor}"))
        .await
        .map_err(|error| describe_sqlstate(&error))
}

fn collect_messages(messages: Vec<SimpleQueryMessage>, caps: Caps) -> ResultSet {
    let mut columns: Vec<Column> = Vec::new();
    let mut collector: Option<Collector> = None;
    let mut affected = None;
    let mut truncated_rows = false;
    for message in messages {
        match message {
            SimpleQueryMessage::RowDescription(description) => {
                columns = description
                    .iter()
                    .map(|column| Column {
                        name: column.name().to_owned(),
                        type_name: "text".to_owned(),
                    })
                    .collect();
                collector = Some(Collector::new(columns.clone()));
            }
            SimpleQueryMessage::Row(row) => {
                let values: Vec<Option<String>> = (0..row.len())
                    .map(|index| row.get(index).map(str::to_owned))
                    .collect();
                let target = collector.get_or_insert_with(|| Collector::new(columns.clone()));
                if !target.push(caps, values) {
                    truncated_rows = true;
                }
            }
            SimpleQueryMessage::CommandComplete(count) => affected = Some(count),
            _ => {}
        }
    }
    let mut result = collector
        .unwrap_or_else(|| Collector::new(Vec::new()))
        .finish(None, None);
    if truncated_rows {
        result.truncated = true;
    }
    result.rows_affected = affected;
    result
}

async fn guarded_statement(
    client: &tokio_postgres::Client,
    sql: &str,
    caps: Caps,
) -> Result<ResultSet> {
    client
        .batch_execute(&format!("SAVEPOINT {SAVEPOINT}"))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let messages = match client.simple_query(sql).await {
        Ok(messages) => messages,
        Err(error) => {
            rollback_savepoint(client).await;
            return Err(describe_sqlstate(&error));
        }
    };
    client
        .batch_execute(&format!("RELEASE SAVEPOINT {SAVEPOINT}"))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    Ok(collect_messages(messages, caps))
}

async fn timed<F, Fut>(
    client: &tokio_postgres::Client,
    timeout: Option<Duration>,
    inside_transaction: bool,
    run: F,
) -> Result<ResultSet>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ResultSet>>,
{
    let Some(timeout) = timeout else {
        return run().await;
    };
    let set_keyword = if inside_transaction {
        "SET LOCAL"
    } else {
        "SET"
    };
    client
        .batch_execute(&format!(
            "{set_keyword} statement_timeout = '{}ms'",
            timeout.as_millis()
        ))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let result = run().await;
    if !inside_transaction && !client.is_closed() {
        let reset = client.batch_execute("RESET statement_timeout").await;
        if let Err(error) = reset {
            tracing::warn!(%error, "statement_timeout could not be reset");
        }
    }
    result
}

async fn autocommit_statement(
    client: &tokio_postgres::Client,
    sql: &str,
    caps: Caps,
) -> Result<ResultSet> {
    let messages = client
        .simple_query(sql)
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    Ok(collect_messages(messages, caps))
}

async fn run_then_rollback(
    client: &tokio_postgres::Client,
    transaction_prefix: Option<&str>,
    sql: &str,
    caps: Caps,
) -> Result<ResultSet> {
    client
        .batch_execute(&begin_statement(transaction_prefix))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let result = guarded_statement(client, sql, caps).await;
    let rolled_back = client.batch_execute("ROLLBACK").await;
    if let Err(error) = rolled_back
        && !client.is_closed()
    {
        return Err(describe_sqlstate(&error));
    }
    result
}

async fn page_from_cursor(
    client: &tokio_postgres::Client,
    cursor: &str,
    pending: &mut VecDeque<Vec<Option<String>>>,
    columns: Vec<Column>,
    caps: Caps,
) -> Result<(Collector, bool)> {
    let wanted = caps.row_cap + 1;
    let mut exhausted = false;
    while pending.len() < wanted && !exhausted {
        let requested = wanted - pending.len();
        let batch = fetch_rows(client, cursor, requested).await?;
        let mut received = 0usize;
        for message in batch {
            if let SimpleQueryMessage::Row(row) = message {
                received += 1;
                pending.push_back(
                    (0..row.len())
                        .map(|index| row.get(index).map(str::to_owned))
                        .collect(),
                );
            }
        }
        if received < requested {
            exhausted = true;
        }
    }
    let mut collector = Collector::new(columns);
    while let Some(row) = pending.front() {
        if collector.rows.len() >= caps.row_cap {
            break;
        }
        let row = row.clone();
        if !collector.push(caps, row) {
            break;
        }
        pending.pop_front();
    }
    let more = !pending.is_empty();
    Ok((collector, more))
}

async fn estimate_rows(client: &tokio_postgres::Client, sql: &str) -> Option<i64> {
    let messages = client
        .simple_query(&format!("EXPLAIN (FORMAT JSON) {sql}"))
        .await
        .ok()?;
    let text: String = messages
        .into_iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        })
        .collect();
    let plan: serde_json::Value = serde_json::from_str(&text).ok()?;
    plan.pointer("/0/Plan/Plan Rows")
        .and_then(serde_json::Value::as_f64)
        .map(|rows| rows.round() as i64)
}

#[must_use]
pub fn expiry_headroom(handle_expiry: Duration) -> Duration {
    handle_expiry + Duration::from_secs(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feature_map_follows_the_documented_version_gates() {
        let fourteen = Features::from_version(140_000);
        assert!(!fourteen.supports_stat_io());
        assert!(!fourteen.supports_merge());
        assert!(!fourteen.supports_merge_returning());
        assert!(!fourteen.supports_returning_old_new());
        let fifteen = Features::from_version(150_000);
        assert!(fifteen.supports_merge());
        assert!(fifteen.supports_nulls_not_distinct());
        let seventeen = Features::from_version(170_000);
        assert!(seventeen.supports_stat_io());
        assert!(seventeen.supports_merge_returning());
        assert!(seventeen.supports_transaction_timeout());
        assert!(seventeen.supports_maintain_privilege());
        assert!(!fifteen.supports_maintain_privilege());
        assert!(!seventeen.supports_returning_old_new());
        let eighteen = Features::from_version(180_006);
        assert!(eighteen.supports_returning_old_new());
        assert!(eighteen.supports_not_enforced_constraints());
        assert!(eighteen.supports_without_overlaps());
        assert_eq!(eighteen.as_map().len(), 11);
    }

    #[test]
    fn handle_ids_are_sixteen_hex_characters_and_distinct() {
        let first = new_handle_id();
        let second = new_handle_id();
        assert_eq!(first.len(), 16);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn the_server_side_idle_timeout_sits_five_seconds_past_the_handle_expiry() {
        assert_eq!(
            expiry_headroom(Duration::from_secs(60)),
            Duration::from_secs(65)
        );
    }

    #[test]
    fn handle_states_have_stable_names() {
        assert_eq!(HandleState::RolledBack.as_str(), "rolled_back");
        assert_eq!(
            serde_json::to_value(HandleState::Lost).unwrap(),
            serde_json::json!("lost")
        );
    }
}
