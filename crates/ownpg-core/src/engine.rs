use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
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
    pub fn as_map(self) -> BTreeMap<&'static str, bool> {
        BTreeMap::from([
            ("pg_stat_io", self.stat_io()),
            ("merge_returning", self.merge_returning()),
            ("transaction_timeout", self.transaction_timeout()),
            ("pg_stat_checkpointer", self.stat_checkpointer()),
            ("returning_old_new", self.returning_old_new()),
            ("not_enforced_constraints", self.not_enforced_constraints()),
            (
                "virtual_generated_columns",
                self.virtual_generated_columns(),
            ),
        ])
    }
}

#[derive(Debug)]
struct Cursor {
    name: String,
    columns: Vec<Column>,
    estimate: Option<i64>,
    expires_at: Instant,
    pending: std::collections::VecDeque<Vec<Option<String>>>,
}

#[derive(Debug)]
struct Primary {
    session: Session,
    in_transaction: bool,
    cursors: BTreeMap<String, Cursor>,
    next_cursor: u64,
}

pub struct Engine {
    settings: Arc<Settings>,
    connector: Connector,
    primary: Mutex<Primary>,
    cancel: std::sync::Mutex<tokio_postgres::CancelToken>,
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

impl Engine {
    pub async fn start(settings: Arc<Settings>, hints: Hints) -> Result<Self> {
        let connector = Connector::new(Arc::clone(&settings)).with_ssh_hints(hints);
        let session = connector.connect().await?;
        let features = Features::from_version(session.info.server_version_num);
        let cancel = std::sync::Mutex::new(session.cancel.clone());
        let engine = Self {
            settings,
            connector,
            cancel,
            primary: Mutex::new(Primary {
                session,
                in_transaction: false,
                cursors: BTreeMap::new(),
                next_cursor: 0,
            }),
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
        self.primary.lock().await.session.info.clone()
    }

    pub async fn role(&self) -> Result<&RoleProfile> {
        self.role
            .get_or_try_init(|| async {
                let primary = self.primary.lock().await;
                RoleProfile::load(&primary.session.client).await
            })
            .await
    }

    pub async fn is_alive(&self) -> bool {
        self.primary.lock().await.session.is_alive().await
    }

    pub async fn cancel_running_statement(&self) -> Result<()> {
        let cancel = self
            .cancel
            .lock()
            .map_err(|_| Error::ProtocolFailed {
                detail: "the cancel token lock is poisoned".to_owned(),
            })?
            .clone();
        let tls = crate::connect::tls::build(&self.settings.connection, "cancel")?;
        cancel
            .cancel_query(tls.connector)
            .await
            .map_err(|error| Error::ProtocolFailed {
                detail: format!("the cancel request failed: {error}"),
            })
    }

    pub async fn open_cursors(&self) -> Vec<CursorSummary> {
        let primary = self.primary.lock().await;
        let now = Instant::now();
        primary
            .cursors
            .iter()
            .map(|(id, cursor)| CursorSummary {
                id: id.clone(),
                expires_in_seconds: cursor.expires_at.saturating_duration_since(now).as_secs(),
            })
            .collect()
    }

    pub async fn sweep(&self) -> Result<usize> {
        let mut primary = self.primary.lock().await;
        let now = Instant::now();
        let expired: Vec<String> = primary
            .cursors
            .iter()
            .filter(|(_, cursor)| cursor.expires_at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            close_cursor_now(&mut primary, id).await?;
        }
        finish_if_idle(&mut primary).await?;
        Ok(expired.len())
    }

    pub async fn close_cursor(&self, id: &str) -> Result<()> {
        let mut primary = self.primary.lock().await;
        if !primary.cursors.contains_key(id) {
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "unknown or expired".to_owned(),
            });
        }
        close_cursor_now(&mut primary, id).await?;
        finish_if_idle(&mut primary).await
    }

    pub async fn run_read(&self, sql: &str, is_select: bool, caps: Caps) -> Result<ResultSet> {
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        sweep_expired(&mut primary).await?;
        begin_read(&mut primary).await?;
        let outcome = if is_select && primary.cursors.len() < CURSOR_CAP {
            self.read_through_cursor(&mut primary, sql, caps).await
        } else {
            read_direct(&mut primary, sql, caps).await
        };
        match outcome {
            Ok(result) => {
                finish_if_idle(&mut primary).await?;
                Ok(result)
            }
            Err(error) => {
                if primary.session.client.is_closed() {
                    primary.in_transaction = false;
                    primary.cursors.clear();
                } else {
                    let _ = finish_if_idle(&mut primary).await;
                }
                Err(error)
            }
        }
    }

    pub async fn fetch(&self, id: &str, caps: Caps) -> Result<ResultSet> {
        let mut primary = self.primary.lock().await;
        sweep_expired(&mut primary).await?;
        let Some(cursor) = primary.cursors.get_mut(id) else {
            return Err(Error::HandleState {
                handle: id.to_owned(),
                state: "unknown or expired".to_owned(),
            });
        };
        let name = cursor.name.clone();
        let columns = cursor.columns.clone();
        let estimate = cursor.estimate;
        let mut pending = std::mem::take(&mut cursor.pending);
        let page = page_from_cursor(&primary.session, &name, &mut pending, columns, caps).await;
        let (collector, more) = match page {
            Ok(page) => page,
            Err(error) => {
                let _ = rollback_all(&mut primary).await;
                return Err(error);
            }
        };
        if more {
            if let Some(cursor) = primary.cursors.get_mut(id) {
                cursor.pending = pending;
                cursor.expires_at = Instant::now() + self.settings.limits.handle_expiry.value;
            }
            Ok(collector.finish(Some(id.to_owned()), estimate))
        } else {
            close_cursor_now(&mut primary, id).await?;
            finish_if_idle(&mut primary).await?;
            Ok(collector.finish(None, estimate))
        }
    }

    pub fn catalog_rows<'a>(
        &'a self,
        sql: &'a str,
        params: &'a [&'a (dyn ToSql + Sync)],
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<tokio_postgres::Row>>> {
        Box::pin(async move {
            let mut primary = self.primary.lock().await;
            self.ensure_alive(&mut primary).await?;
            let stream = primary
                .session
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
        })
    }

    pub async fn catalog_text(&self, sql: &str, caps: Caps) -> Result<ResultSet> {
        self.run_read(sql, false, caps).await
    }

    async fn ensure_alive(&self, primary: &mut Primary) -> Result<()> {
        if primary.session.is_alive().await {
            return Ok(());
        }
        tracing::warn!("the database connection was lost; reconnecting once");
        let fresh = self.connector.connect().await?;
        if let Ok(mut cancel) = self.cancel.lock() {
            *cancel = fresh.cancel.clone();
        }
        primary.session = fresh;
        primary.in_transaction = false;
        primary.cursors.clear();
        Ok(())
    }

    async fn read_through_cursor(
        &self,
        primary: &mut Primary,
        sql: &str,
        caps: Caps,
    ) -> Result<ResultSet> {
        let client = &primary.session.client;
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
        primary.next_cursor += 1;
        let name = format!("ownpg_cursor_{}", primary.next_cursor);
        let declared = client
            .batch_execute(&format!(
                "DECLARE {name} NO SCROLL CURSOR WITHOUT HOLD FOR {sql}"
            ))
            .await;
        if let Err(error) = declared {
            rollback_savepoint(client).await;
            return Err(describe_sqlstate(&error));
        }
        let mut pending = std::collections::VecDeque::new();
        let page =
            page_from_cursor(&primary.session, &name, &mut pending, columns.clone(), caps).await;
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
            primary.cursors.insert(
                id.clone(),
                Cursor {
                    name,
                    columns,
                    estimate,
                    expires_at: Instant::now() + self.settings.limits.handle_expiry.value,
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
}

fn new_handle_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

async fn begin_read(primary: &mut Primary) -> Result<()> {
    if primary.in_transaction {
        return Ok(());
    }
    primary
        .session
        .client
        .batch_execute("BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY")
        .await
        .map_err(|error| describe_sqlstate(&error))?;
    primary.in_transaction = true;
    Ok(())
}

async fn rollback_savepoint(client: &tokio_postgres::Client) {
    let _ = client
        .batch_execute(&format!(
            "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT}"
        ))
        .await;
}

async fn rollback_all(primary: &mut Primary) -> Result<()> {
    primary.cursors.clear();
    if primary.in_transaction {
        primary.in_transaction = false;
        primary
            .session
            .client
            .batch_execute("ROLLBACK")
            .await
            .map_err(|error| describe_sqlstate(&error))?;
    }
    Ok(())
}

async fn finish_if_idle(primary: &mut Primary) -> Result<()> {
    if primary.cursors.is_empty() && primary.in_transaction {
        rollback_all(primary).await?;
    }
    Ok(())
}

async fn close_cursor_now(primary: &mut Primary, id: &str) -> Result<()> {
    if let Some(cursor) = primary.cursors.remove(id) {
        let closed = primary
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

async fn sweep_expired(primary: &mut Primary) -> Result<()> {
    let now = Instant::now();
    let expired: Vec<String> = primary
        .cursors
        .iter()
        .filter(|(_, cursor)| cursor.expires_at <= now)
        .map(|(id, _)| id.clone())
        .collect();
    for id in expired {
        close_cursor_now(primary, &id).await?;
    }
    Ok(())
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

async fn read_direct(primary: &mut Primary, sql: &str, caps: Caps) -> Result<ResultSet> {
    let client = &primary.session.client;
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
    Ok(result)
}

async fn page_from_cursor(
    session: &Session,
    cursor: &str,
    pending: &mut std::collections::VecDeque<Vec<Option<String>>>,
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

impl Engine {
    pub async fn run_and_rollback(&self, sql: &str, caps: Caps) -> Result<ResultSet> {
        let mut primary = self.primary.lock().await;
        self.ensure_alive(&mut primary).await?;
        sweep_expired(&mut primary).await?;
        if primary.in_transaction {
            return Err(Error::HandleState {
                handle: "read transaction".to_owned(),
                state: "holding open cursors; close them before a statement that needs a write transaction"
                    .to_owned(),
            });
        }
        primary
            .session
            .client
            .batch_execute("BEGIN")
            .await
            .map_err(|error| describe_sqlstate(&error))?;
        let result = read_direct(&mut primary, sql, caps).await;
        let rolled_back = primary.session.client.batch_execute("ROLLBACK").await;
        if let Err(error) = rolled_back
            && !primary.session.client.is_closed()
        {
            return Err(describe_sqlstate(&error));
        }
        result
    }
}

impl Engine {
    pub async fn release_everything(&self) -> Result<()> {
        let mut primary = self.primary.lock().await;
        if primary.session.client.is_closed() {
            primary.in_transaction = false;
            primary.cursors.clear();
            return Ok(());
        }
        rollback_all(&mut primary).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feature_map_follows_the_documented_version_gates() {
        let fourteen = Features::from_version(140_000);
        assert!(!fourteen.stat_io());
        assert!(!fourteen.merge_returning());
        assert!(!fourteen.returning_old_new());
        let seventeen = Features::from_version(170_000);
        assert!(seventeen.stat_io());
        assert!(seventeen.merge_returning());
        assert!(seventeen.transaction_timeout());
        assert!(!seventeen.returning_old_new());
        let eighteen = Features::from_version(180_006);
        assert!(eighteen.returning_old_new());
        assert!(eighteen.not_enforced_constraints());
        assert_eq!(eighteen.as_map().len(), 7);
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
}
