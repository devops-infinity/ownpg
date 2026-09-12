use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, TryStreamExt};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_postgres::SimpleQueryMessage;
use tokio_postgres::types::ToSql;

use crate::config::Settings;
use crate::connect::role::RoleProfile;
use crate::connect::ssh::Hints;
use crate::connect::{Connector, Session, SessionInfo, describe_sqlstate};
use crate::error::{Error, Result};
use crate::shape::{Caps, Collector, Column, ResultSet};

pub const CURSOR_CAP: usize = 8;
const SAVEPOINT: &str = "ownpg_call";

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
    pub const fn stat_io(self) -> bool {
        self.server_version_num >= 160_000
    }

    #[must_use]
    pub const fn merge(self) -> bool {
        self.server_version_num >= 150_000
    }

    #[must_use]
    pub const fn merge_returning(self) -> bool {
        self.server_version_num >= 170_000
    }

    #[must_use]
    pub const fn transaction_timeout(self) -> bool {
        self.server_version_num >= 170_000
    }

    #[must_use]
    pub const fn stat_checkpointer(self) -> bool {
        self.server_version_num >= 170_000
    }

    #[must_use]
    pub const fn returning_old_new(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub const fn not_enforced_constraints(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub const fn virtual_generated_columns(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub const fn nulls_not_distinct(self) -> bool {
        self.server_version_num >= 150_000
    }

    #[must_use]
    pub const fn without_overlaps(self) -> bool {
        self.server_version_num >= 180_000
    }

    #[must_use]
    pub fn as_map(self) -> BTreeMap<&'static str, bool> {
        BTreeMap::from([
            ("pg_stat_io", self.stat_io()),
            ("merge", self.merge()),
            ("merge_returning", self.merge_returning()),
            ("transaction_timeout", self.transaction_timeout()),
            ("pg_stat_checkpointer", self.stat_checkpointer()),
            ("returning_old_new", self.returning_old_new()),
            ("not_enforced_constraints", self.not_enforced_constraints()),
            (
                "virtual_generated_columns",
                self.virtual_generated_columns(),
            ),
            ("nulls_not_distinct", self.nulls_not_distinct()),
            ("without_overlaps", self.without_overlaps()),
        ])
    }
}

#[derive(Debug)]
struct Cursor {
    name: String,
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

#[derive(Debug)]
struct Lane {
    session: Session,
    in_read_transaction: bool,
    cursors: BTreeMap<String, Cursor>,
    next_cursor: u64,
}

impl Lane {
    fn new(session: Session) -> Self {
        Self {
            session,
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
    write: Option<WriteHandle>,
    closed: BTreeMap<String, HandleState>,
}

pub struct Engine {
    settings: Arc<Settings>,
    connector: Connector,
    primary: Mutex<Primary>,
    secondary: Mutex<Option<Lane>>,
    cancel: std::sync::Mutex<Vec<tokio_postgres::CancelToken>>,
    role: tokio::sync::OnceCell<RoleProfile>,
    features: Features,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaneChoice {
    Primary,
    Secondary,
}

impl Engine {
    pub async fn start(settings: Arc<Settings>, hints: Hints) -> Result<Self> {
        let connector = Connector::new(Arc::clone(&settings)).with_ssh_hints(hints);
        let session = connector.connect().await?;
        let features = Features::from_version(session.info.server_version_num);
        let cancel = std::sync::Mutex::new(vec![session.cancel.clone()]);
        let engine = Self {
            settings,
            connector,
            primary: Mutex::new(Primary {
                lane: Lane::new(session),
                write: None,
                closed: BTreeMap::new(),
            }),
            secondary: Mutex::new(None),
            cancel,
            role: tokio::sync::OnceCell::new(),
            features,
        };
        if engine.settings.strict_role.value {
            engine.role().await?.enforce(true)?;
        }
        Ok(engine)
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
        self.primary.lock().await.lane.session.info.clone()
    }

    pub async fn role(&self) -> Result<&RoleProfile> {
        self.role
            .get_or_try_init(|| async {
                let primary = self.primary.lock().await;
                RoleProfile::load(&primary.lane.session.client).await
            })
            .await
    }

    pub async fn is_alive(&self) -> bool {
        self.primary.lock().await.lane.session.is_alive().await
    }

    pub async fn cancel_running_statement(&self) -> Result<()> {
        let tokens = self
            .cancel
            .lock()
            .map_err(|_| Error::ProtocolFailed {
                detail: "the cancel token lock is poisoned".to_owned(),
            })?
            .clone();
        let tls = crate::connect::tls::build(&self.settings.connection, "cancel")?;
        let mut failure = None;
        for token in tokens {
            if let Err(error) = token.cancel_query(tls.connector.clone()).await {
                failure = Some(Error::ProtocolFailed {
                    detail: format!("the cancel request failed: {error}"),
                });
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn remember_cancel(&self, token: tokio_postgres::CancelToken) {
        if let Ok(mut tokens) = self.cancel.lock() {
            tokens.push(token);
            if tokens.len() > 2 {
                tokens.remove(0);
            }
        }
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
        out
    }

    pub async fn sweep(&self) -> Result<usize> {
        let mut swept = 0;
        {
            let mut primary = self.primary.lock().await;
            swept += sweep_expired(&mut primary.lane).await?;
            finish_if_idle(&mut primary.lane).await?;
            self.expire_write_handle(&mut primary).await?;
        }
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            swept += sweep_expired(lane).await?;
            finish_if_idle(lane).await?;
        }
        Ok(swept)
    }

    pub async fn close_cursor(&self, id: &str) -> Result<()> {
        {
            let mut primary = self.primary.lock().await;
            if primary.lane.cursors.contains_key(id) {
                close_cursor_now(&mut primary.lane, id).await?;
                return finish_if_idle(&mut primary.lane).await;
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut()
            && lane.cursors.contains_key(id)
        {
            close_cursor_now(lane, id).await?;
            return finish_if_idle(lane).await;
        }
        Err(Error::HandleState {
            handle: id.to_owned(),
            state: "unknown or expired".to_owned(),
        })
    }

    async fn read_lane(&self) -> Result<LaneChoice> {
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        Ok(if primary.write.is_some() {
            LaneChoice::Secondary
        } else {
            LaneChoice::Primary
        })
    }

    async fn ensure_secondary(&self, slot: &mut Option<Lane>) -> Result<()> {
        let alive = match slot {
            Some(lane) => lane.session.is_alive().await,
            None => false,
        };
        if alive {
            return Ok(());
        }
        tracing::debug!("opening the second connection");
        let session = self.connector.connect().await?;
        self.remember_cancel(session.cancel.clone());
        *slot = Some(Lane::new(session));
        Ok(())
    }

    pub async fn run_read(&self, sql: &str, is_select: bool, caps: Caps) -> Result<ResultSet> {
        let expiry = self.settings.limits.handle_expiry.value;
        match self.read_lane().await? {
            LaneChoice::Primary => {
                let mut primary = self.primary.lock().await;
                self.ensure_alive(&mut primary).await?;
                read_on_lane(&mut primary.lane, sql, is_select, caps, expiry).await
            }
            LaneChoice::Secondary => {
                let mut slot = self.secondary.lock().await;
                self.ensure_secondary(&mut slot).await?;
                let lane = slot.as_mut().ok_or_else(|| Error::ProtocolFailed {
                    detail: "the second connection is missing".to_owned(),
                })?;
                read_on_lane(lane, sql, is_select, caps, expiry).await
            }
        }
    }

    pub async fn fetch(&self, id: &str, caps: Caps) -> Result<ResultSet> {
        let expiry = self.settings.limits.handle_expiry.value;
        {
            let mut primary = self.primary.lock().await;
            sweep_expired(&mut primary.lane).await?;
            if primary.lane.cursors.contains_key(id) {
                return fetch_on_lane(&mut primary.lane, id, caps, expiry).await;
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            sweep_expired(lane).await?;
            if lane.cursors.contains_key(id) {
                return fetch_on_lane(lane, id, caps, expiry).await;
            }
        }
        Err(Error::HandleState {
            handle: id.to_owned(),
            state: "unknown or expired".to_owned(),
        })
    }

    pub fn catalog_rows<'a>(
        &'a self,
        sql: &'a str,
        params: &'a [&'a (dyn ToSql + Sync)],
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<tokio_postgres::Row>>> {
        Box::pin(async move {
            match self.read_lane().await? {
                LaneChoice::Primary => {
                    let mut primary = self.primary.lock().await;
                    self.ensure_alive(&mut primary).await?;
                    query_rows(&primary.lane.session, sql, params).await
                }
                LaneChoice::Secondary => {
                    let mut slot = self.secondary.lock().await;
                    self.ensure_secondary(&mut slot).await?;
                    let lane = slot.as_ref().ok_or_else(|| Error::ProtocolFailed {
                        detail: "the second connection is missing".to_owned(),
                    })?;
                    query_rows(&lane.session, sql, params).await
                }
            }
        })
    }

    pub async fn catalog_text(&self, sql: &str, caps: Caps) -> Result<ResultSet> {
        self.run_read(sql, false, caps).await
    }

    async fn ensure_alive(&self, primary: &mut Primary) -> Result<()> {
        if primary.lane.session.is_alive().await {
            return Ok(());
        }
        tracing::warn!("the database connection was lost; reconnecting once");
        if let Some(handle) = primary.write.take() {
            primary.closed.insert(handle.id, HandleState::Lost);
        }
        let fresh = self.connector.connect().await?;
        self.remember_cancel(fresh.cancel.clone());
        primary.lane = Lane::new(fresh);
        Ok(())
    }

    async fn expire_write_handle(&self, primary: &mut Primary) -> Result<()> {
        let expired = primary
            .write
            .as_ref()
            .is_some_and(|handle| handle.expires_at <= Instant::now());
        if !expired {
            return Ok(());
        }
        if let Some(handle) = primary.write.take() {
            tracing::info!(handle = %handle.id, "the transaction handle expired; rolling back");
            let outcome = primary.lane.session.client.batch_execute("ROLLBACK").await;
            if let Err(error) = outcome
                && !primary.lane.session.client.is_closed()
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
        match primary.write.as_mut() {
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
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        if let Some(handle) = &primary.write {
            return Err(Error::HandleState {
                handle: handle.id.clone(),
                state: "open; commit or roll it back before opening another".to_owned(),
            });
        }
        rollback_all(&mut primary.lane).await?;
        primary
            .lane
            .session
            .client
            .batch_execute("BEGIN")
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let handle = WriteHandle {
            id: new_handle_id(),
            principal: principal.to_owned(),
            savepoints: Vec::new(),
            statements: 0,
            expires_at: Instant::now() + self.settings.limits.handle_expiry.value,
        };
        let info = Self::describe_handle(&handle);
        primary.write = Some(handle);
        Ok(info)
    }

    pub async fn transaction_status(&self, id: &str) -> Result<HandleInfo> {
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        if let Some(handle) = primary.write.as_ref().filter(|handle| handle.id == id) {
            return Ok(Self::describe_handle(handle));
        }
        match primary.closed.get(id) {
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

    pub async fn open_transaction(&self) -> Option<HandleInfo> {
        let primary = self.primary.lock().await;
        primary.write.as_ref().map(Self::describe_handle)
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
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        Self::check_handle(&mut primary, id, principal)?;
        if primary.lane.session.client.is_closed() {
            if let Some(handle) = primary.write.take() {
                primary.closed.insert(handle.id, HandleState::Lost);
            }
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "lost".to_owned(),
            });
        }
        let outcome = primary.lane.session.client.batch_execute(statement).await;
        let Some(handle) = primary.write.take() else {
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
                let final_state = if primary.lane.session.client.is_closed() {
                    HandleState::Lost
                } else {
                    HandleState::RolledBack
                };
                primary.closed.insert(handle.id, final_state);
                Err(describe_sqlstate(&error))
            }
        }
    }

    pub async fn savepoint(&self, id: &str, principal: &str, name: &str) -> Result<HandleInfo> {
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        Self::check_handle(&mut primary, id, principal)?;
        let quoted = crate::render::quote_ident(name);
        primary
            .lane
            .session
            .client
            .batch_execute(&format!("SAVEPOINT {quoted}"))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let expiry = self.settings.limits.handle_expiry.value;
        let handle = Self::check_handle(&mut primary, id, principal)?;
        handle.savepoints.push(name.to_owned());
        handle.expires_at = Instant::now() + expiry;
        Ok(Self::describe_handle(handle))
    }

    pub async fn rollback_to(&self, id: &str, principal: &str, name: &str) -> Result<HandleInfo> {
        let mut primary = self.primary.lock().await;
        self.expire_write_handle(&mut primary).await?;
        {
            let handle = Self::check_handle(&mut primary, id, principal)?;
            if !handle.savepoints.iter().any(|known| known == name) {
                return Err(Error::HandleState {
                    handle: id.to_owned(),
                    state: format!("holding no savepoint named `{name}`"),
                });
            }
        }
        let quoted = crate::render::quote_ident(name);
        primary
            .lane
            .session
            .client
            .batch_execute(&format!("ROLLBACK TO SAVEPOINT {quoted}"))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let expiry = self.settings.limits.handle_expiry.value;
        let handle = Self::check_handle(&mut primary, id, principal)?;
        if let Some(position) = handle.savepoints.iter().position(|known| known == name) {
            handle.savepoints.truncate(position + 1);
        }
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
        if let Some(id) = handle {
            if outside_transaction {
                return Err(Error::StatementRefused {
                    rule: "this statement cannot run inside a transaction block; call it without a transaction handle".to_owned(),
                    mode: self.settings.mode.value.to_string(),
                });
            }
            let mut primary = self.primary.lock().await;
            self.expire_write_handle(&mut primary).await?;
            Self::check_handle(&mut primary, id, principal)?;
            if primary.lane.session.client.is_closed() {
                if let Some(handle) = primary.write.take() {
                    primary.closed.insert(handle.id, HandleState::Lost);
                }
                return Err(Error::HandleState {
                    handle: id.to_owned(),
                    state: "lost".to_owned(),
                });
            }
            let result = guarded_statement(&primary.lane.session, sql, caps).await;
            let expiry = self.settings.limits.handle_expiry.value;
            if primary.lane.session.client.is_closed() {
                if let Some(handle) = primary.write.take() {
                    primary.closed.insert(handle.id, HandleState::Lost);
                }
            } else if let Ok(handle) = Self::check_handle(&mut primary, id, principal) {
                handle.statements += 1;
                handle.expires_at = Instant::now() + expiry;
            }
            return result;
        }
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        if let Some(open) = &primary.write {
            return Err(Error::HandleState {
                handle: open.id.clone(),
                state: "open; pass it as `transaction`, or commit or roll it back first".to_owned(),
            });
        }
        sweep_expired(&mut primary.lane).await?;
        finish_if_idle(&mut primary.lane).await?;
        if !primary.lane.in_read_transaction {
            return autocommit_statement(&primary.lane.session, sql, caps).await;
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
        autocommit_statement(&lane.session, sql, caps).await
    }

    pub async fn copy_in(
        &self,
        sql: &str,
        data: &[u8],
        principal: &str,
        handle: Option<&str>,
    ) -> Result<u64> {
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        match (handle, &primary.write) {
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
        let client = &primary.lane.session.client;
        let sink = client
            .copy_in(sql)
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let mut sink = std::pin::pin!(sink);
        sink.send(bytes::Bytes::copy_from_slice(data))
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let rows = sink
            .finish()
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        if let Some(id) = handle
            && let Ok(open) = Self::check_handle(&mut primary, id, principal)
        {
            open.statements += 1;
            open.expires_at = Instant::now() + self.settings.limits.handle_expiry.value;
        }
        Ok(rows)
    }

    pub async fn copy_out(&self, sql: &str, byte_cap: usize) -> Result<(Vec<u8>, bool)> {
        let choice = self.read_lane().await?;
        let mut primary = self.primary.lock().await;
        let mut slot = self.secondary.lock().await;
        let session = match choice {
            LaneChoice::Primary => {
                self.ensure_alive(&mut primary).await?;
                &primary.lane.session
            }
            LaneChoice::Secondary => {
                self.ensure_secondary(&mut slot).await?;
                &slot
                    .as_ref()
                    .ok_or_else(|| Error::ProtocolFailed {
                        detail: "the second connection is missing".to_owned(),
                    })?
                    .session
            }
        };
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

    pub async fn run_and_rollback(&self, sql: &str, caps: Caps) -> Result<ResultSet> {
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        self.expire_write_handle(&mut primary).await?;
        if let Some(open) = &primary.write {
            return Err(Error::HandleState {
                handle: open.id.clone(),
                state: "open; commit or roll it back before an EXPLAIN ANALYZE of a write"
                    .to_owned(),
            });
        }
        sweep_expired(&mut primary.lane).await?;
        finish_if_idle(&mut primary.lane).await?;
        if !primary.lane.in_read_transaction {
            return discard_after(&primary.lane.session, sql, caps).await;
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
        discard_after(&lane.session, sql, caps).await
    }

    pub async fn release_everything(&self) -> Result<()> {
        {
            let mut primary = self.primary.lock().await;
            if let Some(handle) = primary.write.take() {
                if !primary.lane.session.client.is_closed() {
                    let _ = primary.lane.session.client.batch_execute("ROLLBACK").await;
                }
                primary.closed.insert(handle.id, HandleState::RolledBack);
            }
            if primary.lane.session.client.is_closed() {
                primary.lane.forget_state();
            } else {
                rollback_all(&mut primary.lane).await?;
            }
        }
        if let Some(lane) = self.secondary.lock().await.as_mut() {
            if lane.session.client.is_closed() {
                lane.forget_state();
            } else {
                rollback_all(lane).await?;
            }
        }
        Ok(())
    }
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
    session: &Session,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<tokio_postgres::Row>> {
    let stream = session
        .client
        .query_raw(sql, params.iter().copied())
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
    sql: &str,
    is_select: bool,
    caps: Caps,
    expiry: Duration,
) -> Result<ResultSet> {
    sweep_expired(lane).await?;
    begin_read(lane).await?;
    let outcome = if is_select && lane.cursors.len() < CURSOR_CAP {
        read_through_cursor(lane, sql, caps, expiry).await
    } else {
        guarded_statement(&lane.session, sql, caps).await
    };
    match outcome {
        Ok(result) => {
            finish_if_idle(lane).await?;
            Ok(result)
        }
        Err(error) => {
            if lane.session.client.is_closed() {
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
    let page = page_from_cursor(&lane.session, &name, &mut pending, columns, caps).await;
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
    caps: Caps,
    expiry: Duration,
) -> Result<ResultSet> {
    let client = &lane.session.client;
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
    let page = page_from_cursor(&lane.session, &name, &mut pending, columns.clone(), caps).await;
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

async fn begin_read(lane: &mut Lane) -> Result<()> {
    if lane.in_read_transaction {
        return Ok(());
    }
    lane.session
        .client
        .batch_execute("BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY")
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
        lane.session
            .client
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
            .session
            .client
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
    session: &Session,
    cursor: &str,
    count: usize,
) -> Result<Vec<SimpleQueryMessage>> {
    session
        .client
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

async fn guarded_statement(session: &Session, sql: &str, caps: Caps) -> Result<ResultSet> {
    let client = &session.client;
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

async fn autocommit_statement(session: &Session, sql: &str, caps: Caps) -> Result<ResultSet> {
    let messages = session
        .client
        .simple_query(sql)
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    Ok(collect_messages(messages, caps))
}

async fn discard_after(session: &Session, sql: &str, caps: Caps) -> Result<ResultSet> {
    session
        .client
        .batch_execute("BEGIN")
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    let result = guarded_statement(session, sql, caps).await;
    let rolled_back = session.client.batch_execute("ROLLBACK").await;
    if let Err(error) = rolled_back
        && !session.client.is_closed()
    {
        return Err(describe_sqlstate(&error));
    }
    result
}

async fn page_from_cursor(
    session: &Session,
    cursor: &str,
    pending: &mut VecDeque<Vec<Option<String>>>,
    columns: Vec<Column>,
    caps: Caps,
) -> Result<(Collector, bool)> {
    let wanted = caps.row_cap + 1;
    let mut exhausted = false;
    while pending.len() < wanted && !exhausted {
        let requested = wanted - pending.len();
        let batch = fetch_rows(session, cursor, requested).await?;
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
        assert!(!fourteen.stat_io());
        assert!(!fourteen.merge());
        assert!(!fourteen.merge_returning());
        assert!(!fourteen.returning_old_new());
        let fifteen = Features::from_version(150_000);
        assert!(fifteen.merge());
        assert!(fifteen.nulls_not_distinct());
        let seventeen = Features::from_version(170_000);
        assert!(seventeen.stat_io());
        assert!(seventeen.merge_returning());
        assert!(seventeen.transaction_timeout());
        assert!(!seventeen.returning_old_new());
        let eighteen = Features::from_version(180_006);
        assert!(eighteen.returning_old_new());
        assert!(eighteen.not_enforced_constraints());
        assert!(eighteen.without_overlaps());
        assert_eq!(eighteen.as_map().len(), 10);
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
