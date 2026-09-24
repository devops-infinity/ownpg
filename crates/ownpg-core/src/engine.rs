use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
const SAVEPOINT_NAME: &str = "ownpg_call";

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

pub const SUPPORTED_MAJORS: std::ops::RangeInclusive<i32> = 14..=18;

impl Features {
    #[must_use]
    pub const fn from_version(server_version_num: i32) -> Self {
        Self { server_version_num }
    }

    #[must_use]
    pub const fn major(self) -> i32 {
        self.server_version_num.div_euclid(10_000)
    }

    #[must_use]
    pub fn version_warning(self, server_version: &str) -> Option<String> {
        (!SUPPORTED_MAJORS.contains(&self.major())).then(|| {
            format!(
                "PostgreSQL {server_version} is outside the tested range {} to {}",
                SUPPORTED_MAJORS.start(),
                SUPPORTED_MAJORS.end()
            )
        })
    }

    #[must_use]
    pub const fn reports_commit_status(self) -> bool {
        self.server_version_num >= 130_000
    }

    #[must_use]
    pub const fn supports_publication_filters(self) -> bool {
        self.server_version_num >= 150_000
    }

    #[must_use]
    pub const fn supports_publication_generated_columns(self) -> bool {
        self.server_version_num >= 180_000
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    IdleExpiry,
    TransactionTimeout,
    ConnectionLost,
    Shutdown,
}

impl EndReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IdleExpiry => "idle_expiry",
            Self::TransactionTimeout => "transaction_timeout",
            Self::ConnectionLost => "connection_lost",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedHandle {
    pub id: String,
    pub principal: String,
    pub state: HandleState,
    pub reason: EndReason,
}

const ENDED_HANDLE_CAP: usize = 256;

#[derive(Debug)]
struct WriteHandle {
    id: String,
    principal: String,
    savepoints: Vec<String>,
    statements: u64,
    expires_at: Instant,
    lifetime: Option<Duration>,
    lifetime_ends_at: Option<Instant>,
}

impl WriteHandle {
    fn deadline(&self) -> Instant {
        self.lifetime_ends_at
            .map_or(self.expires_at, |end| end.min(self.expires_at))
    }

    fn outlived(&self, now: Instant) -> bool {
        self.lifetime_ends_at.is_some_and(|end| end <= now)
    }

    fn touch(&mut self, idle: Duration, call: Option<(Instant, Duration)>) {
        self.expires_at = Instant::now() + idle;
        if let (Some(lifetime), Some((started, per_call))) = (self.lifetime, call) {
            let rearmed = started + lifetime.max(per_call.saturating_add(TIMEOUT_HEADROOM));
            self.lifetime_ends_at = self.lifetime_ends_at.map(|end| end.max(rearmed));
        }
    }

    fn expiry_reason(&self, now: Instant) -> EndReason {
        if self.outlived(now) {
            tracing::info!(handle = %self.id, "the transaction handle reached transaction_timeout; rolling back");
            EndReason::TransactionTimeout
        } else {
            tracing::info!(handle = %self.id, "the transaction handle expired; rolling back");
            EndReason::IdleExpiry
        }
    }
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
        metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Error> {
        if metrics.age() >= POOLED_MAX_LIFETIME || metrics.last_used() >= POOLED_MAX_IDLE {
            return Err(deadpool::managed::RecycleError::Message(
                "the pooled connection reached its age or idle limit and is replaced".into(),
            ));
        }
        if session.ready_for_reuse().await {
            Ok(())
        } else {
            Err(deadpool::managed::RecycleError::Message(
                "the pooled connection is closed or could not be reset".into(),
            ))
        }
    }
}

pub const POOLED_MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);
pub const POOLED_MAX_IDLE: Duration = Duration::from_secs(10 * 60);

pub type Pool = deadpool::managed::Pool<SessionManager>;
type PooledSession = deadpool::managed::Object<SessionManager>;

fn pool_error(error: deadpool::managed::PoolError<Error>, waited: Duration) -> Error {
    match error {
        deadpool::managed::PoolError::Backend(error) => error,
        deadpool::managed::PoolError::Timeout(deadpool::managed::TimeoutType::Wait) => {
            Error::PoolExhausted {
                waited_seconds: waited.as_secs(),
            }
        }
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

#[derive(Debug, Clone, Copy)]
struct ClosedHandle {
    state: HandleState,
    closed_at: Instant,
}

#[derive(Debug)]
struct Primary {
    lane: Lane,
    write_handle: Option<WriteHandle>,
    closed: BTreeMap<String, ClosedHandle>,
}

#[derive(Debug)]
struct PooledHandle {
    lane: Lane,
    handle: WriteHandle,
}

#[derive(Debug)]
struct PooledSlot {
    principal: String,
    held: Arc<Mutex<PooledHandle>>,
}

#[derive(Debug, Default)]
struct PooledHandles {
    open: HashMap<String, PooledSlot>,
    closed: BTreeMap<String, ClosedHandle>,
}

tokio::task_local! {
    pub static CALL_ID: u64;
}

#[derive(Default)]
struct Running {
    next_seq: u64,
    cancel_tokens: HashMap<u64, Vec<(u64, crate::connect::Canceller)>>,
    cancelled: HashSet<u64>,
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
    progress: Mutex<Option<Lane>>,
    pooled_handles: Mutex<PooledHandles>,
    running: std::sync::Mutex<Running>,
    ended: std::sync::Mutex<VecDeque<EndedHandle>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStatus {
    pub max: usize,
    pub idle: usize,
    pub used: usize,
    pub waiting: usize,
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
            .wait_timeout(Some(settings.connection.connect_timeout.value))
            .create_timeout(Some(settings.connection.connect_timeout.value))
            .recycle_timeout(Some(Duration::from_secs(5)))
            .runtime(deadpool::Runtime::Tokio1)
            .build()
            .map_err(|error| Error::ProtocolFailed {
                detail: format!("the connection pool could not be built: {error}"),
            })?;
            let warm = pool
                .get()
                .await
                .map_err(|error| pool_error(error, settings.connection.connect_timeout.value))?;
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
            progress: Mutex::new(None),
            pooled_handles: Mutex::new(PooledHandles::default()),
            running: std::sync::Mutex::new(Running::default()),
            ended: std::sync::Mutex::new(VecDeque::new()),
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

    #[must_use]
    pub fn pool_status(&self) -> Option<PoolStatus> {
        self.pool.as_ref().map(|pool| {
            let status = pool.status();
            PoolStatus {
                max: status.max_size,
                idle: status.available,
                used: status.size.saturating_sub(status.available),
                waiting: status.waiting,
            }
        })
    }

    async fn checkout(&self) -> Result<Lane> {
        let Some(pool) = &self.pool else {
            return Err(Error::ProtocolFailed {
                detail: "no connection pool is open".to_owned(),
            });
        };
        self.pinned
            .lock()
            .await
            .retain(|lane| !lane.conn.client().is_closed());
        let object = pool
            .get()
            .await
            .map_err(|error| pool_error(error, self.settings.connection.connect_timeout.value))?;
        Ok(Lane::from_conn(Conn::Pooled(object)))
    }

    async fn reuse_pinned(&self) -> Option<Lane> {
        let mut pinned = self.pinned.lock().await;
        let position = pinned
            .iter()
            .position(|lane| lane.cursors.len() < CURSOR_CAP && !lane.conn.client().is_closed())?;
        Some(pinned.remove(position))
    }

    fn note_ended(&self, id: &str, principal: &str, state: HandleState, reason: EndReason) {
        let Ok(mut ended) = self.ended.lock() else {
            tracing::warn!(handle = id, "the ended-handle queue is unavailable");
            return;
        };
        if ended.len() >= ENDED_HANDLE_CAP {
            ended.pop_front();
        }
        ended.push_back(EndedHandle {
            id: id.to_owned(),
            principal: principal.to_owned(),
            state,
            reason,
        });
    }

    #[must_use]
    pub fn take_ended_handles(&self) -> Vec<EndedHandle> {
        self.ended
            .lock()
            .map(|mut ended| ended.drain(..).collect())
            .unwrap_or_default()
    }

    fn note_closed(
        &self,
        closed: &mut BTreeMap<String, ClosedHandle>,
        id: String,
        state: HandleState,
    ) {
        let now = Instant::now();
        let keep = self.settings.limits.handle_expiry.value;
        closed.retain(|_, entry| now.saturating_duration_since(entry.closed_at) < keep);
        closed.insert(
            id,
            ClosedHandle {
                state,
                closed_at: now,
            },
        );
    }

    #[must_use]
    pub fn settings(&self) -> &Arc<Settings> {
        &self.settings
    }

    #[must_use]
    pub const fn features(&self) -> Features {
        self.features
    }

    fn commit_probe(&self) -> Option<&Connector> {
        self.features
            .reports_commit_status()
            .then_some(self.connector.as_ref())
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
        let Some(pool) = &self.pool else {
            return self.primary.lock().await.lane.conn.is_alive().await;
        };
        if pool.get().await.is_err() {
            return false;
        }
        if let Ok(mut primary) = self.primary.try_lock()
            && let Err(error) = self.ensure_alive(&mut primary).await
        {
            tracing::warn!(%error, "the startup connection could not be reopened");
        }
        true
    }

    pub async fn cancel_call(&self, call: u64) -> Result<()> {
        let cancel_tokens = {
            let mut running = self.running.lock().map_err(|_| Error::ProtocolFailed {
                detail: "the running-call lock is poisoned".to_owned(),
            })?;
            running.cancelled.insert(call);
            running
                .cancel_tokens
                .get(&call)
                .map(|cancel_tokens| {
                    cancel_tokens
                        .iter()
                        .map(|(_, token)| token.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        self.send_cancel(cancel_tokens).await
    }

    pub fn forget_call(&self, call: u64) {
        if let Ok(mut running) = self.running.lock() {
            running.cancelled.remove(&call);
        }
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

    async fn send_cancel(&self, cancellers: Vec<crate::connect::Canceller>) -> Result<()> {
        let mut failure = None;
        for canceller in cancellers {
            if let Err(error) = canceller.cancel().await {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn track(&self, session: &Session) -> Result<Tracked<'_>> {
        let Ok(call) = CALL_ID.try_with(|id| *id) else {
            return Ok(Tracked {
                engine: self,
                key: None,
            });
        };
        let mut running = self.running.lock().map_err(|_| Error::ProtocolFailed {
            detail: "the running-call lock is poisoned".to_owned(),
        })?;
        if running.cancelled.contains(&call) {
            return Err(Error::CallCancelled);
        }
        running.next_seq = running.next_seq.saturating_add(1);
        let seq = running.next_seq;
        running
            .cancel_tokens
            .entry(call)
            .or_default()
            .push((seq, session.canceller()));
        Ok(Tracked {
            engine: self,
            key: Some((call, seq)),
        })
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
        let mut failures = Vec::new();
        let write_open = {
            let mut primary = self.primary.lock().await;
            match sweep_expired(&mut primary.lane).await {
                Ok(count) => swept += count,
                Err(error) => failures.push(error),
            }
            if let Err(error) = finish_if_idle(&mut primary.lane).await {
                failures.push(error);
            }
            if let Err(error) = self.expire_write_handle(&mut primary).await {
                failures.push(error);
            }
            primary.write_handle.is_some()
        };
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            match sweep_expired(lane).await {
                Ok(count) => swept += count,
                Err(error) => failures.push(error),
            }
            if let Err(error) = finish_if_idle(lane).await {
                failures.push(error);
            }
        }
        if !write_open {
            self.drop_idle_secondary().await;
        }
        let mut pinned = self.pinned.lock().await;
        swept += prune_pinned(&mut pinned).await;
        drop(pinned);
        let mut handles = self.pooled_handles.lock().await;
        swept += self.pooled_sweep(&mut handles).await;
        drop(handles);
        match failures.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(swept),
        }
    }

    pub fn sweep_interval(&self) -> Duration {
        let cursor = self.settings.limits.cursor_expiry.value;
        let handle = self.settings.limits.handle_expiry.value;
        (cursor.min(handle) / 2).clamp(Duration::from_secs(1), SWEEP_CEILING)
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

    async fn ensure_progress_lane(&self, slot: &mut Option<Lane>) -> Result<()> {
        let alive = match slot {
            Some(lane) => !lane.conn.client().is_closed(),
            None => false,
        };
        if alive {
            return Ok(());
        }
        tracing::debug!("opening the progress connection");
        let session = self.connector.connect().await?;
        *slot = Some(Lane::new(session));
        Ok(())
    }

    pub async fn progress_rows(
        &self,
        sql: &str,
        params: &[&dyn CatalogParam],
    ) -> Result<Vec<tokio_postgres::Row>> {
        if self.pool.is_some() {
            let mut lane = self.checkout().await?;
            let _tracked = self.track(lane.conn.session())?;
            return catalog_on_lane(&mut lane, self.transaction_prefix(), sql, params).await;
        }
        let mut slot = self.progress.lock().await;
        self.ensure_progress_lane(&mut slot).await?;
        let lane = slot.as_mut().ok_or_else(|| Error::ProtocolFailed {
            detail: "the progress connection is missing".to_owned(),
        })?;
        let _tracked = self.track(lane.conn.session())?;
        catalog_on_lane(lane, self.transaction_prefix(), sql, params).await
    }

    pub async fn drop_progress_lane(&self) {
        *self.progress.lock().await = None;
    }

    fn transaction_prefix(&self) -> Option<&str> {
        self.transaction_prefix.as_deref()
    }

    fn timeout_plan(&self, per_call: Option<Duration>) -> TimeoutPlan {
        TimeoutPlan {
            per_call,
            transaction_timeout: self
                .features
                .supports_transaction_timeout()
                .then_some(self.settings.limits.transaction_timeout.value),
            behind_pooler: self.settings.connection.pooled.value == Some(true),
        }
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
            if let Some(lane) = self.reuse_pinned().await {
                return Ok(LaneHold::Pooled(Box::new(lane)));
            }
            return Ok(LaneHold::Pooled(Box::new(self.checkout().await?)));
        }
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        self.ensure_alive(&mut primary).await?;
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
        let _tracked = self.track(lane.conn.session())?;
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
        let _tracked = self.track(lane.conn.session())?;
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
                let _tracked = self.track(primary.lane.conn.session())?;
                return fetch_on_lane(&mut primary.lane, id, caps, expiry).await;
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            sweep_expired(lane).await?;
            if lane.cursors.contains_key(id) {
                check_cursor_owner(lane, id, principal)?;
                let _tracked = self.track(lane.conn.session())?;
                return fetch_on_lane(lane, id, caps, expiry).await;
            }
        }
        let mut pinned = self.pinned.lock().await;
        prune_pinned(&mut pinned).await;
        if let Some(position) = pinned.iter().position(|lane| lane.cursors.contains_key(id)) {
            if let Some(lane) = pinned.get(position) {
                check_cursor_owner(lane, id, principal)?;
            }
            let mut lane = pinned.remove(position);
            let _tracked = self.track(lane.conn.session())?;
            let result = fetch_on_lane(&mut lane, id, caps, expiry).await;
            if !lane.cursors.is_empty() {
                pinned.push(lane);
            }
            return result;
        }
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
            let _tracked = self.track(lane.conn.session())?;
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
            self.note_ended(
                &handle.id,
                &handle.principal,
                HandleState::Lost,
                EndReason::ConnectionLost,
            );
            self.note_closed(&mut primary.closed, handle.id, HandleState::Lost);
        }
        let fresh = self.connector.connect().await?;
        primary.lane = Lane::new(fresh);
        Ok(())
    }

    async fn expire_write_handle(&self, primary: &mut Primary) -> Result<()> {
        let now = Instant::now();
        let expired = primary
            .write_handle
            .as_ref()
            .is_some_and(|handle| handle.deadline() <= now);
        if !expired {
            return Ok(());
        }
        if let Some(handle) = primary.write_handle.take() {
            let reason = handle.expiry_reason(now);
            self.note_ended(&handle.id, &handle.principal, HandleState::Expired, reason);
            self.note_closed(&mut primary.closed, handle.id, HandleState::Expired);
            if primary.lane.conn.client().is_closed() {
                return Ok(());
            }
            primary
                .lane
                .conn
                .client()
                .batch_execute("ROLLBACK")
                .await
                .map_err(|error| describe_sqlstate(&error))?;
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
                    return Err(owned_by_another(id));
                }
                Ok(handle)
            }
            _ => {
                let state = primary
                    .closed
                    .get(id)
                    .map_or("unknown", |entry| entry.state.as_str());
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
                .deadline()
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
        sweep_expired(&mut primary.lane).await?;
        finish_if_idle(&mut primary.lane).await?;
        if !primary.lane.cursors.is_empty() {
            let open: Vec<&str> = primary.lane.cursors.keys().map(String::as_str).collect();
            return Err(Error::HandleState {
                handle: "cursors".to_owned(),
                state: format!(
                    "open on this connection ({}); fetch them to the end, close them, or let them expire before opening a transaction",
                    open.join(", ")
                ),
            });
        }
        rollback_all(&mut primary.lane).await?;
        let _tracked = self.track(primary.lane.conn.session())?;
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
        let now = Instant::now();
        let lifetime = Some(self.settings.limits.transaction_timeout.value)
            .filter(|lifetime| !lifetime.is_zero());
        WriteHandle {
            id: new_handle_id(),
            principal: principal.to_owned(),
            savepoints: Vec::new(),
            statements: 0,
            expires_at: now + self.settings.limits.handle_expiry.value,
            lifetime,
            lifetime_ends_at: lifetime.map(|lifetime| now + lifetime),
        }
    }

    pub async fn transaction_status(&self, id: &str, principal: &str) -> Result<HandleInfo> {
        if self.pool.is_some() {
            return self.pooled_status(id, principal).await;
        }
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        if primary.lane.conn.client().is_closed()
            && let Some(handle) = primary.write_handle.take()
        {
            self.note_ended(
                &handle.id,
                &handle.principal,
                HandleState::Lost,
                EndReason::ConnectionLost,
            );
            self.note_closed(&mut primary.closed, handle.id, HandleState::Lost);
        }
        if let Some(handle) = primary
            .write_handle
            .as_ref()
            .filter(|handle| handle.id == id)
        {
            if handle.principal != principal {
                return Err(owned_by_another(id));
            }
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
            let _tracked = self.track(primary.lane.conn.session())?;
            if primary.lane.conn.client().is_closed() {
                if let Some(handle) = primary.write_handle.take() {
                    self.note_closed(&mut primary.closed, handle.id, HandleState::Lost);
                }
                return Err(lost_handle(id));
            }
            let outcome = finish_block(
                primary.lane.conn.client(),
                self.commit_probe(),
                id,
                statement,
            )
            .await;
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
                    self.note_closed(&mut primary.closed, handle.id, state);
                    Ok(info)
                }
                Err(error) => {
                    let final_state = if primary.lane.conn.client().is_closed() {
                        HandleState::Lost
                    } else {
                        HandleState::RolledBack
                    };
                    self.note_closed(&mut primary.closed, handle.id, final_state);
                    Err(error)
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
                    |_| Ok(()),
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
        let _tracked = self.track(primary.lane.conn.session())?;
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
        handle.touch(expiry, None);
        Ok(Self::describe_handle(handle))
    }

    pub async fn rollback_to(&self, id: &str, principal: &str, name: &str) -> Result<HandleInfo> {
        let quoted = crate::render::quote_ident(name);
        let statement = format!("ROLLBACK TO SAVEPOINT {quoted}");
        if self.pool.is_some() {
            return self
                .pooled_on_handle(
                    id,
                    principal,
                    |handle| check_savepoint(handle, id, name),
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
        let _tracked = self.track(primary.lane.conn.session())?;
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
        handle.touch(expiry, None);
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
        self.run_write_reporting_pid(
            sql,
            caps,
            principal,
            handle,
            outside_transaction,
            timeout,
            None,
        )
        .await
    }

    pub async fn run_write_reporting_pid(
        &self,
        sql: &str,
        caps: Caps,
        principal: &str,
        handle: Option<&str>,
        outside_transaction: bool,
        timeout: Option<Duration>,
        backend_pid: Option<&std::sync::atomic::AtomicI32>,
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
            let _tracked = self.track(primary.lane.conn.session())?;
            if primary.lane.conn.client().is_closed() {
                if let Some(handle) = primary.write_handle.take() {
                    self.note_ended(
                        &handle.id,
                        &handle.principal,
                        HandleState::Lost,
                        EndReason::ConnectionLost,
                    );
                    self.note_closed(&mut primary.closed, handle.id, HandleState::Lost);
                }
                return Err(lost_handle(id));
            }
            report_backend_pid(primary.lane.conn.client(), backend_pid).await;
            let call = timeout.map(|per_call| (Instant::now(), per_call));
            let result = timed(
                primary.lane.conn.session(),
                self.timeout_plan(timeout),
                true,
                || guarded_statement(primary.lane.conn.client(), sql, caps),
            )
            .await;
            let expiry = self.settings.limits.handle_expiry.value;
            if primary.lane.conn.client().is_closed() {
                if let Some(handle) = primary.write_handle.take() {
                    self.note_ended(
                        &handle.id,
                        &handle.principal,
                        HandleState::Lost,
                        EndReason::ConnectionLost,
                    );
                    self.note_closed(&mut primary.closed, handle.id, HandleState::Lost);
                }
            } else if let Ok(handle) = Self::check_handle(&mut primary, id, principal) {
                if result.is_ok() {
                    handle.statements += 1;
                }
                handle.touch(expiry, call);
            }
            return result;
        }
        if self.pool.is_some() {
            let lane = self.checkout().await?;
            let _tracked = self.track(lane.conn.session())?;
            report_backend_pid(lane.conn.client(), backend_pid).await;
            return autocommit(
                lane.conn.session(),
                self.commit_probe(),
                self.transaction_prefix(),
                self.timeout_plan(timeout),
                outside_transaction,
                sql,
                caps,
            )
            .await;
        }
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        self.ensure_alive(&mut primary).await?;
        if let Some(open) = &primary.write_handle {
            return Err(Error::HandleState {
                handle: open.id.clone(),
                state: "open; pass it as `transaction`, or commit or roll it back first".to_owned(),
            });
        }
        sweep_expired(&mut primary.lane).await?;
        finish_if_idle(&mut primary.lane).await?;
        if !primary.lane.in_read_transaction {
            let _tracked = self.track(primary.lane.conn.session())?;
            report_backend_pid(primary.lane.conn.client(), backend_pid).await;
            return autocommit(
                primary.lane.conn.session(),
                self.commit_probe(),
                self.transaction_prefix(),
                self.timeout_plan(timeout),
                outside_transaction,
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
        let _tracked = self.track(lane.conn.session())?;
        report_backend_pid(lane.conn.client(), backend_pid).await;
        autocommit(
            lane.conn.session(),
            self.commit_probe(),
            self.transaction_prefix(),
            self.timeout_plan(timeout),
            outside_transaction,
            sql,
            caps,
        )
        .await
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
        self.expire_write_handle(&mut primary).await?;
        self.ensure_alive(&mut primary).await?;
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
        let _tracked = self.track(primary.lane.conn.session())?;
        let rows = copy_in_on(
            primary.lane.conn.client(),
            self.commit_probe(),
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
            open.touch(self.settings.limits.handle_expiry.value, None);
        }
        Ok(rows)
    }

    pub async fn copy_out(&self, sql: &str, byte_cap: usize) -> Result<(Vec<u8>, bool)> {
        let mut hold = self.read_hold().await?;
        let lane = hold.lane()?;
        let _tracked = self.track(lane.conn.session())?;
        let result = copy_out_on_lane(lane, self.transaction_prefix(), sql, byte_cap).await;
        self.release_hold(hold).await;
        result
    }

    pub async fn run_and_rollback(&self, sql: &str, caps: Caps) -> Result<ResultSet> {
        if self.pool.is_some() {
            let lane = self.checkout().await?;
            let _tracked = self.track(lane.conn.session())?;
            return run_then_rollback(lane.conn.client(), self.transaction_prefix(), sql, caps)
                .await;
        }
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        self.ensure_alive(&mut primary).await?;
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
            let _tracked = self.track(primary.lane.conn.session())?;
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
        let _tracked = self.track(lane.conn.session())?;
        run_then_rollback(lane.conn.client(), self.transaction_prefix(), sql, caps).await
    }

    pub async fn release_everything(&self) -> Result<()> {
        let mut failure = None;
        {
            let mut primary = self.primary.lock().await;
            if let Some(handle) = primary.write_handle.take() {
                if !primary.lane.conn.client().is_closed() {
                    let _ = primary.lane.conn.client().batch_execute("ROLLBACK").await;
                }
                self.note_ended(
                    &handle.id,
                    &handle.principal,
                    HandleState::RolledBack,
                    EndReason::Shutdown,
                );
                self.note_closed(&mut primary.closed, handle.id, HandleState::RolledBack);
            }
            if primary.lane.conn.client().is_closed() {
                primary.lane.forget_state();
            } else if let Err(error) = rollback_all(&mut primary.lane).await {
                failure = Some(error);
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            if lane.conn.client().is_closed() {
                lane.forget_state();
            } else if let Err(error) = rollback_all(lane).await {
                failure.get_or_insert(error);
            }
        }
        *self.progress.lock().await = None;
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
            let open: Vec<(String, PooledSlot)> = handles.open.drain().collect();
            for (id, slot) in open {
                let held = slot.held.lock().await;
                if !held.lane.conn.client().is_closed() {
                    let _ = held.lane.conn.client().batch_execute("ROLLBACK").await;
                }
                self.note_ended(
                    &id,
                    &slot.principal,
                    HandleState::RolledBack,
                    EndReason::Shutdown,
                );
                self.note_closed(&mut handles.closed, id, HandleState::RolledBack);
            }
        }
        if let Some(pool) = &self.pool {
            pool.close();
        }
        failure.map_or(Ok(()), Err)
    }

    async fn pooled_sweep(&self, handles: &mut PooledHandles) -> usize {
        let now = Instant::now();
        let mut expired = Vec::new();
        for (id, slot) in &handles.open {
            if let Ok(guard) = Arc::clone(&slot.held).try_lock_owned()
                && guard.handle.deadline() <= now
            {
                expired.push((id.clone(), guard));
            }
        }
        let count = expired.len();
        for (id, guard) in expired {
            handles.open.remove(&id);
            let reason = guard.handle.expiry_reason(now);
            roll_back_quietly(guard.lane.conn.client()).await;
            self.note_ended(&id, &guard.handle.principal, HandleState::Expired, reason);
            self.note_closed(&mut handles.closed, id, HandleState::Expired);
        }
        count
    }

    fn open_handle_for(handles: &PooledHandles, principal: &str) -> Option<String> {
        handles
            .open
            .iter()
            .find(|(_, slot)| slot.principal == principal)
            .map(|(id, _)| id.clone())
    }

    async fn pooled_begin(&self, principal: &str) -> Result<HandleInfo> {
        {
            let mut handles = self.pooled_handles.lock().await;
            self.pooled_sweep(&mut handles).await;
            if let Some(open) = Self::open_handle_for(&handles, principal) {
                return Err(second_handle(open));
            }
            if let Some(pool) = &self.pool
                && handles.open.len() >= handle_cap(pool.status().max_size)
            {
                return Err(Error::HandleState {
                    handle: "pool".to_owned(),
                    state: format!(
                        "at its limit; {} of the {} pooled connections hold open transaction handles, and the rest stay free for other calls",
                        handles.open.len(),
                        pool.status().max_size
                    ),
                });
            }
        }
        let lane = self.checkout().await?;
        let _tracked = self.track(lane.conn.session())?;
        lane.conn
            .client()
            .batch_execute(&begin_statement(self.transaction_prefix()))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let handle = self.new_write_handle(principal);
        let info = Self::describe_handle(&handle);
        let mut handles = self.pooled_handles.lock().await;
        if let Some(open) = Self::open_handle_for(&handles, principal) {
            roll_back_quietly(lane.conn.client()).await;
            return Err(second_handle(open));
        }
        handles.open.insert(
            handle.id.clone(),
            PooledSlot {
                principal: principal.to_owned(),
                held: Arc::new(Mutex::new(PooledHandle { lane, handle })),
            },
        );
        Ok(info)
    }

    async fn pooled_status(&self, id: &str, principal: &str) -> Result<HandleInfo> {
        let held = {
            let mut handles = self.pooled_handles.lock().await;
            self.pooled_sweep(&mut handles).await;
            match handles.open.get(id) {
                Some(slot) if slot.principal == principal => Arc::clone(&slot.held),
                Some(_) => return Err(owned_by_another(id)),
                None => return closed_handle_info(&handles.closed, id),
            }
        };
        let guard = held.lock().await;
        if guard.lane.conn.client().is_closed() {
            drop(guard);
            self.pooled_lose(id).await;
            return Err(lost_handle(id));
        }
        Ok(Self::describe_handle(&guard.handle))
    }

    async fn pooled_lookup(&self, id: &str, principal: &str) -> Result<Arc<Mutex<PooledHandle>>> {
        let mut handles = self.pooled_handles.lock().await;
        self.pooled_sweep(&mut handles).await;
        match handles.open.get(id) {
            Some(slot) if slot.principal == principal => Ok(Arc::clone(&slot.held)),
            Some(_) => Err(owned_by_another(id)),
            None => Err(Error::HandleState {
                handle: id.to_owned(),
                state: handles
                    .closed
                    .get(id)
                    .map_or("unknown", |entry| entry.state.as_str())
                    .to_owned(),
            }),
        }
    }

    async fn pooled_lose(&self, id: &str) {
        let mut handles = self.pooled_handles.lock().await;
        if let Some(slot) = handles.open.remove(id) {
            self.note_ended(
                id,
                &slot.principal,
                HandleState::Lost,
                EndReason::ConnectionLost,
            );
            self.note_closed(&mut handles.closed, id.to_owned(), HandleState::Lost);
        }
    }

    async fn pooled_finish(
        &self,
        id: &str,
        principal: &str,
        statement: &str,
        state: HandleState,
    ) -> Result<HandleInfo> {
        let held = {
            let mut handles = self.pooled_handles.lock().await;
            self.pooled_sweep(&mut handles).await;
            match handles.open.get(id) {
                Some(slot) if slot.principal == principal => {}
                Some(_) => return Err(owned_by_another(id)),
                None => {
                    return Err(Error::HandleState {
                        handle: id.to_owned(),
                        state: handles
                            .closed
                            .get(id)
                            .map_or("unknown", |entry| entry.state.as_str())
                            .to_owned(),
                    });
                }
            }
            handles.open.remove(id).map(|slot| slot.held)
        };
        let Some(held) = held else {
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "unknown".to_owned(),
            });
        };
        let guard = held.lock().await;
        if guard.lane.conn.client().is_closed() {
            drop(guard);
            let mut handles = self.pooled_handles.lock().await;
            self.note_closed(&mut handles.closed, id.to_owned(), HandleState::Lost);
            return Err(lost_handle(id));
        }
        let _tracked = self.track(guard.lane.conn.session())?;
        let outcome =
            finish_block(guard.lane.conn.client(), self.commit_probe(), id, statement).await;
        let mut info = Self::describe_handle(&guard.handle);
        let closed = guard.lane.conn.client().is_closed();
        drop(guard);
        let mut handles = self.pooled_handles.lock().await;
        match outcome {
            Ok(()) => {
                info.state = state;
                info.expires_in_seconds = 0;
                self.note_closed(&mut handles.closed, id.to_owned(), state);
                Ok(info)
            }
            Err(error) => {
                let final_state = if closed {
                    HandleState::Lost
                } else {
                    HandleState::RolledBack
                };
                self.note_closed(&mut handles.closed, id.to_owned(), final_state);
                Err(error)
            }
        }
    }

    async fn pooled_on_handle<C, F>(
        &self,
        id: &str,
        principal: &str,
        check: C,
        update: F,
        statement: &str,
    ) -> Result<HandleInfo>
    where
        C: FnOnce(&WriteHandle) -> Result<()>,
        F: FnOnce(&mut WriteHandle),
    {
        let held = self.pooled_lookup(id, principal).await?;
        let mut guard = held.lock().await;
        check(&guard.handle)?;
        if guard.lane.conn.client().is_closed() {
            drop(guard);
            self.pooled_lose(id).await;
            return Err(lost_handle(id));
        }
        let _tracked = self.track(guard.lane.conn.session())?;
        guard
            .lane
            .conn
            .client()
            .batch_execute(statement)
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        update(&mut guard.handle);
        guard
            .handle
            .touch(self.settings.limits.handle_expiry.value, None);
        Ok(Self::describe_handle(&guard.handle))
    }

    async fn pooled_run_in_handle(
        &self,
        id: &str,
        principal: &str,
        sql: &str,
        caps: Caps,
        timeout: Option<Duration>,
    ) -> Result<ResultSet> {
        let held = self.pooled_lookup(id, principal).await?;
        let mut guard = held.lock().await;
        if guard.lane.conn.client().is_closed() {
            drop(guard);
            self.pooled_lose(id).await;
            return Err(lost_handle(id));
        }
        let _tracked = self.track(guard.lane.conn.session())?;
        let client = guard.lane.conn.client();
        let call = timeout.map(|per_call| (Instant::now(), per_call));
        let result = timed(
            guard.lane.conn.session(),
            self.timeout_plan(timeout),
            true,
            || guarded_statement(client, sql, caps),
        )
        .await;
        if guard.lane.conn.client().is_closed() {
            drop(guard);
            self.pooled_lose(id).await;
        } else {
            if result.is_ok() {
                guard.handle.statements += 1;
            }
            guard
                .handle
                .touch(self.settings.limits.handle_expiry.value, call);
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
            let _tracked = self.track(lane.conn.session())?;
            return copy_in_on(
                lane.conn.client(),
                self.commit_probe(),
                self.transaction_prefix(),
                false,
                sql,
                data,
            )
            .await;
        };
        let held = self.pooled_lookup(id, principal).await?;
        let mut guard = held.lock().await;
        if guard.lane.conn.client().is_closed() {
            drop(guard);
            self.pooled_lose(id).await;
            return Err(lost_handle(id));
        }
        let _tracked = self.track(guard.lane.conn.session())?;
        let rows = copy_in_on(
            guard.lane.conn.client(),
            self.commit_probe(),
            self.transaction_prefix(),
            true,
            sql,
            data,
        )
        .await?;
        guard.handle.statements += 1;
        guard
            .handle
            .touch(self.settings.limits.handle_expiry.value, None);
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

const fn handle_cap(pool_size: usize) -> usize {
    if pool_size > 1 { pool_size - 1 } else { 1 }
}

fn begin_statement(transaction_prefix: Option<&str>) -> String {
    match transaction_prefix {
        Some(transaction_prefix) => format!("BEGIN; {transaction_prefix}"),
        None => "BEGIN".to_owned(),
    }
}

fn owned_by_another(id: &str) -> Error {
    Error::HandleState {
        handle: id.to_owned(),
        state: "owned by another principal".to_owned(),
    }
}

fn lost_handle(id: &str) -> Error {
    Error::HandleState {
        handle: id.to_owned(),
        state: "lost".to_owned(),
    }
}

fn second_handle(open: String) -> Error {
    Error::HandleState {
        handle: open,
        state: "open; commit or roll it back before opening another".to_owned(),
    }
}

async fn report_backend_pid(
    client: &tokio_postgres::Client,
    backend_pid: Option<&std::sync::atomic::AtomicI32>,
) {
    let Some(slot) = backend_pid else {
        return;
    };
    match query_backend_pid(client).await {
        Ok(pid) => slot.store(pid, std::sync::atomic::Ordering::Relaxed),
        Err(error) => tracing::debug!(%error, "the backend pid could not be read"),
    }
}

async fn query_backend_pid(client: &tokio_postgres::Client) -> Result<i32> {
    let rows = query_rows(client, "SELECT pg_catalog.pg_backend_pid()", &[]).await?;
    rows.first()
        .map(|row| row.try_get::<_, i32>(0))
        .transpose()
        .map_err(|error| describe_sqlstate(&error))?
        .ok_or_else(|| Error::ProtocolFailed {
            detail: "the backend pid could not be read".to_owned(),
        })
}

const TRANSACTION_ID_SQL: &str = "SELECT pg_catalog.pg_current_xact_id_if_assigned()::text";
const LOST_COMMIT_CHECKS: usize = 10;
const LOST_COMMIT_WAIT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommitResult {
    Committed,
    NeverSent,
    RolledBack { transaction: String },
    Unknown { transaction: Option<String> },
}

impl CommitResult {
    fn unknown_error(transaction: Option<String>) -> Error {
        Error::CommitOutcomeUnknown {
            operation: transaction.map_or_else(
                || "COMMIT".to_owned(),
                |transaction| format!("COMMIT of transaction {transaction}"),
            ),
        }
    }
}

fn connection_lost(client: &tokio_postgres::Client, error: &tokio_postgres::Error) -> bool {
    match error.as_db_error() {
        Some(db) => matches!(
            db.parsed_severity(),
            Some(tokio_postgres::error::Severity::Fatal | tokio_postgres::error::Severity::Panic)
        ),
        None => client.is_closed() || error.is_closed(),
    }
}

async fn commit_checked(
    client: &tokio_postgres::Client,
    probe: Option<&Connector>,
) -> Result<CommitResult> {
    let check = if probe.is_some() {
        TRANSACTION_ID_SQL
    } else {
        "SELECT 1"
    };
    let transaction = match client.simple_query(check).await {
        Ok(messages) => messages.into_iter().find_map(|message| match message {
            SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        }),
        Err(error) if connection_lost(client, &error) => return Ok(CommitResult::NeverSent),
        Err(error) => return Err(describe_sqlstate(&error)),
    };
    let failure = match client.batch_execute("COMMIT").await {
        Ok(()) => return Ok(CommitResult::Committed),
        Err(error) => error,
    };
    if !connection_lost(client, &failure) {
        return Err(describe_sqlstate(&failure));
    }
    Ok(match (probe, transaction) {
        (Some(connector), Some(transaction)) => lost_commit_outcome(connector, transaction).await,
        (Some(_), None) => CommitResult::Committed,
        (None, _) => CommitResult::Unknown { transaction: None },
    })
}

async fn lost_commit_outcome(connector: &Connector, transaction: String) -> CommitResult {
    let session = match connector.connect().await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(%error, transaction, "the commit outcome could not be checked");
            return CommitResult::Unknown {
                transaction: Some(transaction),
            };
        }
    };
    for _ in 0..LOST_COMMIT_CHECKS {
        let status = session
            .client
            .query_one(
                "SELECT pg_catalog.pg_xact_status($1::text::xid8)::text",
                &[&transaction],
            )
            .await
            .map(|row| row.try_get::<_, Option<String>>(0).ok().flatten());
        match status.as_ref().map(Option::as_deref) {
            Ok(Some("committed")) => {
                tracing::warn!(
                    transaction,
                    "the connection closed during COMMIT, and the server reports the transaction committed"
                );
                return CommitResult::Committed;
            }
            Ok(Some("aborted")) => return CommitResult::RolledBack { transaction },
            Ok(_) => tokio::time::sleep(LOST_COMMIT_WAIT).await,
            Err(error) => {
                tracing::warn!(%error, transaction, "the commit outcome could not be checked");
                break;
            }
        }
    }
    CommitResult::Unknown {
        transaction: Some(transaction),
    }
}

async fn finish_block(
    client: &tokio_postgres::Client,
    probe: Option<&Connector>,
    id: &str,
    statement: &str,
) -> Result<()> {
    if statement != "COMMIT" {
        return client
            .batch_execute(statement)
            .await
            .map_err(|error| describe_sqlstate(&error));
    }
    let rolled_back = |state: String| Error::HandleState {
        handle: id.to_owned(),
        state,
    };
    match commit_checked(client, probe).await {
        Ok(CommitResult::Committed) => Ok(()),
        Ok(CommitResult::NeverSent) => Err(rolled_back(
            "rolled back: the connection closed before COMMIT was sent, so nothing was committed"
                .to_owned(),
        )),
        Ok(CommitResult::RolledBack { transaction }) => Err(rolled_back(format!(
            "rolled back: the connection closed during COMMIT and the server aborted transaction {transaction}, so nothing was committed"
        ))),
        Ok(CommitResult::Unknown { transaction }) => Err(CommitResult::unknown_error(transaction)),
        Err(error) => {
            roll_back_quietly(client).await;
            if block_aborted(&error) {
                return Err(rolled_back(
                    "aborted by a failed statement; it was rolled back and nothing was committed"
                        .to_owned(),
                ));
            }
            Err(error)
        }
    }
}

async fn roll_back_quietly(client: &tokio_postgres::Client) {
    if client.is_closed() {
        return;
    }
    if let Err(error) = client.batch_execute("ROLLBACK").await {
        tracing::warn!(%error, "the transaction could not be rolled back after a failed check");
    }
}

fn closed_handle_info(closed: &BTreeMap<String, ClosedHandle>, id: &str) -> Result<HandleInfo> {
    match closed.get(id) {
        Some(entry) => Ok(HandleInfo {
            id: id.to_owned(),
            state: entry.state,
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
    if let Some(position) = handle.savepoints.iter().rposition(|known| known == name) {
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
        if lane.in_read_transaction {
            let client = lane.conn.client();
            let outcome = guarded(client, || query_rows(client, sql, params)).await;
            settle_lane(lane, &outcome).await;
            return outcome;
        }
        return query_rows(lane.conn.client(), sql, params).await;
    }
    sweep_expired(lane).await?;
    begin_read(lane, transaction_prefix).await?;
    let client = lane.conn.client();
    let outcome = guarded(client, || query_rows(client, sql, params)).await;
    settle_lane(lane, &outcome).await;
    outcome
}

fn block_aborted(error: &Error) -> bool {
    matches!(error, Error::SqlFailed { sqlstate: Some(code), .. } if code == "25P02")
}

async fn settle_lane<T>(lane: &mut Lane, outcome: &Result<T>) {
    if lane.conn.client().is_closed() {
        lane.forget_state();
        return;
    }
    match outcome {
        Ok(_) => {
            if let Err(error) = finish_if_idle(lane).await {
                tracing::debug!(%error, "the read transaction could not be finished");
            }
        }
        Err(error) if block_aborted(error) => {
            let _ = rollback_all(lane).await;
        }
        Err(_) => {
            let _ = finish_if_idle(lane).await;
        }
    }
}

async fn guarded<T, F, Fut>(client: &tokio_postgres::Client, run: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    client
        .batch_execute(&format!("SAVEPOINT {SAVEPOINT_NAME}"))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    match run().await {
        Ok(value) => {
            client
                .batch_execute(&format!("RELEASE SAVEPOINT {SAVEPOINT_NAME}"))
                .await
                .map_err(|error| describe_sqlstate(&error))?;
            Ok(value)
        }
        Err(error) => {
            rollback_savepoint(client).await;
            Err(error)
        }
    }
}

async fn autocommit(
    session: &Session,
    probe: Option<&Connector>,
    transaction_prefix: Option<&str>,
    plan: TimeoutPlan,
    outside_transaction: bool,
    sql: &str,
    caps: Caps,
) -> Result<ResultSet> {
    let client = &session.client;
    let Some(transaction_prefix) = transaction_prefix.filter(|_| !outside_transaction) else {
        return timed(session, plan, false, || {
            autocommit_statement(client, sql, caps)
        })
        .await;
    };
    client
        .batch_execute(&format!("BEGIN; {transaction_prefix}"))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let result = timed(session, plan, true, || {
        autocommit_statement(client, sql, caps)
    })
    .await;
    settle_transaction(client, probe, result.is_ok()).await?;
    result
}

async fn settle_transaction(
    client: &tokio_postgres::Client,
    probe: Option<&Connector>,
    commit: bool,
) -> Result<()> {
    if !commit {
        if !client.is_closed()
            && let Err(error) = client.batch_execute("ROLLBACK").await
        {
            tracing::debug!(%error, "the rollback after a failed statement did not complete");
        }
        return Ok(());
    }
    match commit_checked(client, probe).await? {
        CommitResult::Committed => Ok(()),
        CommitResult::NeverSent => Err(Error::ProtocolFailed {
            detail: "the connection closed before COMMIT was sent, so the statement was rolled back and nothing was committed".to_owned(),
        }),
        CommitResult::RolledBack { transaction } => Err(Error::ProtocolFailed {
            detail: format!(
                "the connection closed during COMMIT and the server rolled back transaction {transaction}, so nothing was committed"
            ),
        }),
        CommitResult::Unknown { transaction } => Err(CommitResult::unknown_error(transaction)),
    }
}

async fn copy_in_on(
    client: &tokio_postgres::Client,
    probe: Option<&Connector>,
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
    if inside_handle {
        client
            .batch_execute(&format!("SAVEPOINT {SAVEPOINT_NAME}"))
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
    if inside_handle {
        if result.is_ok() {
            client
                .batch_execute(&format!("RELEASE SAVEPOINT {SAVEPOINT_NAME}"))
                .await
                .map_err(|error| describe_sqlstate(&error))?;
        } else {
            rollback_savepoint(client).await;
        }
    }
    if wrap.is_some() {
        settle_transaction(client, probe, result.is_ok()).await?;
    }
    result
}

async fn copy_out_on_lane(
    lane: &mut Lane,
    transaction_prefix: Option<&str>,
    sql: &str,
    byte_cap: usize,
) -> Result<(Vec<u8>, bool)> {
    sweep_expired(lane).await?;
    begin_read(lane, transaction_prefix).await?;
    let session = lane.conn.session();
    let outcome = guarded(&session.client, || async {
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
    })
    .await;
    settle_lane(lane, &outcome).await;
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
    if owner.is_some() && lane.cursors.len() >= CURSOR_CAP {
        return Err(Error::HandleState {
            handle: "cursors".to_owned(),
            state: format!(
                "at the cap of {CURSOR_CAP} open cursors on this connection; fetch one to the end, close it, or let it expire"
            ),
        });
    }
    begin_read(lane, transaction_prefix).await?;
    let outcome = match owner {
        Some(principal) => read_through_cursor(lane, sql, principal, caps, expiry).await,
        None => guarded_statement(lane.conn.client(), sql, caps)
            .await
            .map(|mut result| {
                result.rows_affected = None;
                result
            }),
    };
    settle_lane(lane, &outcome).await;
    outcome
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
    let shared = lane.cursors.len() > 1;
    let client = lane.conn.client();
    let page = if shared {
        guarded(client, || {
            page_from_cursor(client, &name, &mut pending, columns, caps)
        })
        .await
    } else {
        page_from_cursor(client, &name, &mut pending, columns, caps).await
    };
    let (collector, more) = match page {
        Ok(page) => page,
        Err(error) if shared && !lane.conn.client().is_closed() => {
            close_cursor_now(lane, id).await?;
            return Err(error);
        }
        Err(error) => {
            if let Err(cleanup) = rollback_all(lane).await {
                tracing::debug!(error = %cleanup, "the read transaction could not be rolled back after a failed fetch");
            }
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
        .batch_execute(&format!("SAVEPOINT {SAVEPOINT_NAME}"))
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
        .batch_execute(&format!("RELEASE SAVEPOINT {SAVEPOINT_NAME}"))
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
    let rolled_back = client
        .batch_execute(&format!(
            "ROLLBACK TO SAVEPOINT {SAVEPOINT_NAME}; RELEASE SAVEPOINT {SAVEPOINT_NAME}"
        ))
        .await;
    if let Err(error) = rolled_back {
        tracing::debug!(%error, "the savepoint could not be rolled back");
    }
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

async fn prune_pinned(pinned: &mut Vec<Lane>) -> usize {
    let mut swept = 0;
    for lane in pinned.iter_mut() {
        if lane.conn.client().is_closed() {
            lane.forget_state();
            continue;
        }
        match sweep_expired(lane).await {
            Ok(count) => swept += count,
            Err(error) => {
                tracing::debug!(%error, "a pinned cursor could not be closed");
                lane.forget_state();
                continue;
            }
        }
        if let Err(error) = finish_if_idle(lane).await {
            tracing::debug!(%error, "a pinned read transaction could not be finished");
            lane.forget_state();
        }
    }
    pinned.retain(|lane| !lane.cursors.is_empty() && !lane.conn.client().is_closed());
    swept
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

pub(crate) const SIMPLE_QUERY_TYPE: &str = "unknown";

struct MessageShaper {
    caps: Caps,
    columns: Vec<Column>,
    collector: Option<Collector>,
    affected: Option<u64>,
    truncated_rows: bool,
}

impl MessageShaper {
    const fn new(caps: Caps) -> Self {
        Self {
            caps,
            columns: Vec::new(),
            collector: None,
            affected: None,
            truncated_rows: false,
        }
    }

    fn push(&mut self, message: SimpleQueryMessage) {
        match message {
            SimpleQueryMessage::RowDescription(description) => {
                self.columns = description
                    .iter()
                    .map(|column| Column {
                        name: column.name().to_owned(),
                        type_name: SIMPLE_QUERY_TYPE.to_owned(),
                    })
                    .collect();
                self.collector = Some(Collector::new(self.columns.clone()));
            }
            SimpleQueryMessage::Row(row) => {
                let values: Vec<Option<String>> = (0..row.len())
                    .map(|index| row.get(index).map(str::to_owned))
                    .collect();
                let columns = &self.columns;
                let target = self
                    .collector
                    .get_or_insert_with(|| Collector::new(columns.clone()));
                if !target.push(self.caps, values) {
                    self.truncated_rows = true;
                }
            }
            SimpleQueryMessage::CommandComplete(count) => self.affected = Some(count),
            _ => {}
        }
    }

    fn finish(self) -> ResultSet {
        let mut result = self
            .collector
            .unwrap_or_else(|| Collector::new(Vec::new()))
            .finish(None, None);
        if self.truncated_rows {
            result.truncated = true;
        }
        result.rows_affected = self.affected;
        result
    }
}

pub(crate) fn collect_messages(messages: Vec<SimpleQueryMessage>, caps: Caps) -> ResultSet {
    let mut shaper = MessageShaper::new(caps);
    for message in messages {
        shaper.push(message);
    }
    shaper.finish()
}

async fn collect_stream(
    stream: tokio_postgres::SimpleQueryStream,
    caps: Caps,
) -> std::result::Result<ResultSet, tokio_postgres::Error> {
    let mut stream = std::pin::pin!(stream);
    let mut shaper = MessageShaper::new(caps);
    while let Some(message) = stream.try_next().await? {
        shaper.push(message);
    }
    Ok(shaper.finish())
}

async fn guarded_statement(
    client: &tokio_postgres::Client,
    sql: &str,
    caps: Caps,
) -> Result<ResultSet> {
    guarded(client, || autocommit_statement(client, sql, caps)).await
}

const TIMEOUT_HEADROOM: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
struct TimeoutPlan {
    per_call: Option<Duration>,
    transaction_timeout: Option<Duration>,
    behind_pooler: bool,
}

impl TimeoutPlan {
    fn raise_statements(self, per_call: Duration, keyword: &str) -> Vec<String> {
        let mut statements = vec![format!(
            "{keyword} statement_timeout = '{}ms'",
            per_call.as_millis()
        )];
        if let Some(configured) = self.transaction_timeout.filter(|value| !value.is_zero()) {
            let raised = configured.max(per_call.saturating_add(TIMEOUT_HEADROOM));
            statements.push(format!("{keyword} transaction_timeout = 0"));
            statements.push(format!(
                "{keyword} transaction_timeout = '{}ms'",
                raised.as_millis()
            ));
        }
        statements
    }

    fn restore_statements(self) -> Vec<String> {
        let mut statements = vec!["RESET statement_timeout".to_owned()];
        if let Some(configured) = self.transaction_timeout {
            statements.push(format!(
                "SET transaction_timeout = '{}ms'",
                configured.as_millis()
            ));
        }
        statements
    }
}

async fn timed<F, Fut>(
    session: &Session,
    plan: TimeoutPlan,
    inside_transaction: bool,
    run: F,
) -> Result<ResultSet>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ResultSet>>,
{
    let Some(per_call) = plan.per_call else {
        return run().await;
    };
    let client = &session.client;
    if plan.behind_pooler && !inside_transaction {
        return cancel_after(session, per_call, run()).await;
    }
    let keyword = if inside_transaction {
        "SET LOCAL"
    } else {
        "SET"
    };
    client
        .batch_execute(&plan.raise_statements(per_call, keyword).join("; "))
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let result = run().await;
    if !inside_transaction && !client.is_closed() {
        let restored = client
            .batch_execute(&plan.restore_statements().join("; "))
            .await;
        if let Err(error) = restored {
            tracing::warn!(%error, "the session timeouts could not be restored");
        }
    }
    result
}

async fn cancel_after<Fut>(session: &Session, limit: Duration, run: Fut) -> Result<ResultSet>
where
    Fut: Future<Output = Result<ResultSet>>,
{
    let mut run = std::pin::pin!(run);
    tokio::select! {
        result = &mut run => result,
        () = tokio::time::sleep(limit) => {
            if let Err(error) = session.cancel_running_statement().await {
                tracing::warn!(%error, "the statement passed its timeout but the cancel request failed");
            }
            run.await
        }
    }
}

async fn autocommit_statement(
    client: &tokio_postgres::Client,
    sql: &str,
    caps: Caps,
) -> Result<ResultSet> {
    let stream = client
        .simple_query_raw(sql)
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    collect_stream(stream, caps)
        .await
        .map_err(|error| describe_sqlstate(&error))
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
    while collector.rows.len() < caps.row_cap {
        let Some(row) = pending.pop_front() else {
            break;
        };
        if let Some(refused) = collector.offer(caps, row) {
            pending.push_front(refused);
            break;
        }
    }
    let more = !pending.is_empty();
    Ok((collector, more))
}

async fn estimate_rows(client: &tokio_postgres::Client, sql: &str) -> Option<i64> {
    let messages = guarded(client, || async {
        client
            .simple_query(&format!("EXPLAIN (FORMAT JSON) {sql}"))
            .await
            .map_err(|error| describe_sqlstate(&error))
    })
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

const SWEEP_CEILING: Duration = Duration::from_secs(4);

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
        assert!(Features::from_version(130_000).reports_commit_status());
        assert!(!Features::from_version(120_022).reports_commit_status());
    }

    async fn live_connector() -> Option<(Connector, tempfile::TempDir)> {
        let dsn = std::env::var("OWNPG_TEST_DSN")
            .ok()
            .filter(|value| !value.trim().is_empty())?;
        let dir = tempfile::tempdir().unwrap();
        let env = crate::config::Environment::new(
            BTreeMap::from([("OWNPG_DSN".to_owned(), dsn)]),
            None,
            None,
        );
        let (settings, _) = crate::config::resolve(
            crate::config::FlagLayer::default(),
            crate::config::Sources {
                env: &env,
                paths: crate::config::AppPaths::from_base(
                    dir.path().join("config"),
                    dir.path().join("data"),
                ),
                keychain: None,
            },
        )
        .unwrap();
        Some((Connector::new(Arc::new(settings)), dir))
    }

    async fn finished_transaction(connector: &Connector, end: &str) -> String {
        let session = connector.connect().await.unwrap();
        session.client.batch_execute("BEGIN").await.unwrap();
        let transaction: String = session
            .client
            .query_one("SELECT pg_catalog.pg_current_xact_id()::text", &[])
            .await
            .unwrap()
            .get(0);
        session.client.batch_execute(end).await.unwrap();
        transaction
    }

    #[tokio::test]
    async fn a_lost_commit_is_settled_from_the_server_transaction_status() {
        let Some((connector, _dir)) = live_connector().await else {
            return;
        };
        let committed = finished_transaction(&connector, "COMMIT").await;
        assert_eq!(
            lost_commit_outcome(&connector, committed).await,
            CommitResult::Committed
        );
        let aborted = finished_transaction(&connector, "ROLLBACK").await;
        assert_eq!(
            lost_commit_outcome(&connector, aborted.clone()).await,
            CommitResult::RolledBack {
                transaction: aborted
            }
        );
    }

    #[tokio::test]
    async fn a_commit_after_a_failed_statement_rolls_back_and_says_so() {
        let Some((connector, _dir)) = live_connector().await else {
            return;
        };
        let session = connector.connect().await.unwrap();
        session.client.batch_execute("BEGIN").await.unwrap();
        session
            .client
            .batch_execute("SELECT 1/0")
            .await
            .unwrap_err();
        let error = finish_block(&session.client, Some(&connector), "h", "COMMIT")
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("aborted by a failed statement"),
            "{error}"
        );
        let status: String = session
            .client
            .query_one(
                "SELECT pg_catalog.pg_current_xact_id_if_assigned() IS NULL",
                &[],
            )
            .await
            .map(|row| row.get::<_, bool>(0).to_string())
            .unwrap();
        assert_eq!(status, "true");
    }

    #[tokio::test]
    async fn a_lost_commit_still_in_progress_stays_unknown_and_names_the_transaction() {
        let Some((connector, _dir)) = live_connector().await else {
            return;
        };
        let open = connector.connect().await.unwrap();
        open.client.batch_execute("BEGIN").await.unwrap();
        let transaction: String = open
            .client
            .query_one("SELECT pg_catalog.pg_current_xact_id()::text", &[])
            .await
            .unwrap()
            .get(0);
        let outcome = lost_commit_outcome(&connector, transaction.clone()).await;
        assert_eq!(
            outcome,
            CommitResult::Unknown {
                transaction: Some(transaction.clone())
            }
        );
        let error = CommitResult::unknown_error(Some(transaction.clone()));
        assert_eq!(error.id(), crate::error::ErrorId::CommitOutcomeUnknown);
        assert!(error.to_string().contains(&transaction), "{error}");
        open.client.batch_execute("ROLLBACK").await.unwrap();
    }

    fn handle(idle: Duration, lifetime: Option<Duration>) -> WriteHandle {
        let now = Instant::now();
        WriteHandle {
            id: "h".to_owned(),
            principal: "p".to_owned(),
            savepoints: Vec::new(),
            statements: 0,
            expires_at: now + idle,
            lifetime,
            lifetime_ends_at: lifetime.map(|lifetime| now + lifetime),
        }
    }

    #[test]
    fn a_handle_ends_at_the_earlier_of_its_idle_expiry_and_its_lifetime() {
        let busy = handle(Duration::from_secs(60), Some(Duration::from_secs(5)));
        assert!(busy.deadline() <= Instant::now() + Duration::from_secs(5));
        assert!(!busy.outlived(Instant::now()));
        assert!(busy.outlived(Instant::now() + Duration::from_secs(6)));
        let idle = handle(Duration::from_secs(2), Some(Duration::from_secs(300)));
        assert!(idle.deadline() <= Instant::now() + Duration::from_secs(2));
        let unbounded = handle(Duration::from_secs(60), None);
        assert!(!unbounded.outlived(Instant::now() + Duration::from_secs(3600)));
    }

    #[test]
    fn touching_a_handle_refreshes_the_idle_expiry_but_not_the_lifetime() {
        let mut touched = handle(Duration::from_secs(1), Some(Duration::from_secs(10)));
        let lifetime_end = touched.lifetime_ends_at;
        touched.touch(Duration::from_secs(60), None);
        assert_eq!(touched.lifetime_ends_at, lifetime_end);
        assert!(touched.expires_at > Instant::now() + Duration::from_secs(30));
        assert!(touched.deadline() <= Instant::now() + Duration::from_secs(10));
    }

    #[test]
    fn a_call_with_a_longer_timeout_extends_the_lifetime_as_the_server_rearms_it() {
        let mut long = handle(Duration::from_secs(60), Some(Duration::from_secs(10)));
        let started = Instant::now();
        long.touch(
            Duration::from_secs(60),
            Some((started, Duration::from_secs(120))),
        );
        assert_eq!(
            long.lifetime_ends_at,
            Some(started + Duration::from_secs(125))
        );
        let mut short = handle(Duration::from_secs(60), Some(Duration::from_secs(300)));
        let before = short.lifetime_ends_at;
        short.touch(
            Duration::from_secs(60),
            Some((started, Duration::from_secs(1))),
        );
        assert!(short.lifetime_ends_at >= before);
    }

    #[test]
    fn a_disabled_transaction_timeout_is_never_armed_by_a_per_call_timeout() {
        let plan = TimeoutPlan {
            per_call: Some(Duration::from_secs(30)),
            transaction_timeout: Some(Duration::ZERO),
            behind_pooler: false,
        };
        let statements = plan.raise_statements(Duration::from_secs(30), "SET LOCAL");
        assert_eq!(statements, vec!["SET LOCAL statement_timeout = '30000ms'"]);
        let armed = TimeoutPlan {
            transaction_timeout: Some(Duration::from_secs(10)),
            ..plan
        };
        let statements = armed.raise_statements(Duration::from_secs(30), "SET LOCAL");
        assert_eq!(
            statements.last().map(String::as_str),
            Some("SET LOCAL transaction_timeout = '35000ms'")
        );
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
    fn handle_states_have_stable_names() {
        assert_eq!(HandleState::RolledBack.as_str(), "rolled_back");
        assert_eq!(
            serde_json::to_value(HandleState::Lost).unwrap(),
            serde_json::json!("lost")
        );
    }
}
