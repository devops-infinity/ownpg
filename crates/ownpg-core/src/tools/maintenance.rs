use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::catalog;
use super::confirm::{Verdict, ask};
use super::ddl::{Toggle, number, scoped_name};
use super::read::facts_for;
use super::write::dry_run_reply;
use super::{AuditFacts, Call, Outcome, Route, ToolFailure, ToolOutput, route, text_rows};
use crate::classify::{self, SchemaScope};
use crate::error::{Error, Result};
use crate::render::{QualifiedName, ident_list, quote_ident, verify};
use crate::shape::{ResultSet, UNTRUSTED_NOTICE};
use crate::tool_specs;

const VACUUM_DESCRIPTION: &str = "Run VACUUM on tables of the scoped schema (every table when none is named), with full, freeze, analyze, disable_page_skipping, skip_locked, index_cleanup, truncate, parallel workers, and a buffer usage limit, or run CHECKPOINT with operation checkpoint. VACUUM runs outside any transaction under its own statement timeout and reports progress from pg_stat_progress_vacuum when the client sent a progress token. VACUUM FULL rewrites the table under an exclusive lock and needs confirm: true or the confirmation prompt.";

const ANALYZE_DESCRIPTION: &str = "Run ANALYZE on tables of the scoped schema (every table when none is named), optionally on named columns of one table, with skip_locked and its own statement timeout.";

const REINDEX_DESCRIPTION: &str = "Rebuild one index, every index of a table, or every index in the scoped schema with REINDEX, concurrently when asked (which runs outside any transaction), under its own statement timeout.";

const REFRESH_DESCRIPTION: &str = "Refresh a materialized view in the scoped schema, concurrently when a unique index allows it, with or without data, under its own statement timeout.";

const VACUUM_NEEDS_DESCRIPTION: &str = "Report which tables in the scoped schema need a vacuum or an analyze, using the autovacuum threshold formula (threshold + scale factor times live rows) with per-table reloptions applied, alongside dead and live row counts and the last vacuum and analyze times. Sorted by dead rows, largest first, then name.";

const BACKEND_DESCRIPTION: &str = "Cancel the running statement of a backend with pg_cancel_backend, or terminate the backend with pg_terminate_backend. Termination drops that session's connection and needs confirm: true or the confirmation prompt. Both work through pg_signal_backend rules: the target must belong to the same role or the connected role must be a member of pg_signal_backend.";

fn default_statement_timeout() -> u64 {
    600
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VacuumOperation {
    #[default]
    Vacuum,
    Checkpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexCleanup {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VacuumArgs {
    #[serde(default)]
    pub operation: VacuumOperation,
    #[serde(default)]
    #[schemars(description = "Tables to vacuum; empty means every table in the scoped schema.")]
    pub tables: Vec<String>,
    #[serde(default)]
    pub full: bool,
    #[serde(default)]
    pub freeze: bool,
    #[serde(default)]
    pub analyze: bool,
    #[serde(default)]
    pub disable_page_skipping: bool,
    #[serde(default)]
    pub skip_locked: bool,
    #[serde(default)]
    pub index_cleanup: IndexCleanup,
    #[serde(default)]
    pub truncate: Toggle,
    #[serde(default)]
    #[schemars(description = "Parallel workers as text; empty leaves the default.")]
    pub parallel: String,
    #[serde(default)]
    #[schemars(
        description = "BUFFER_USAGE_LIMIT such as 256MB (PostgreSQL 16 and later); empty leaves the default."
    )]
    pub buffer_usage_limit: String,
    #[serde(default = "default_statement_timeout")]
    #[schemars(description = "Statement timeout in seconds for this run (default 600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
}

async fn scoped_tables(call: &Call) -> Result<Vec<QualifiedName>> {
    let scoped = call.settings().schema.value.clone();
    let rows = call
        .engine()
        .catalog_rows(
            "SELECT c.relname::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r', 'm') ORDER BY c.relname",
            &[&scoped],
        )
        .await?;
    let mut out = Vec::new();
    for row in &rows {
        let name: String = catalog::read_column(row, 0)?;
        out.push(QualifiedName {
            schema: scoped.clone(),
            name,
        });
    }
    Ok(out)
}

async fn table_list(call: &Call, names: &[String]) -> Result<Vec<QualifiedName>> {
    if names.is_empty() {
        return scoped_tables(call).await;
    }
    names
        .iter()
        .map(|name| scoped_name(call, "tables", name))
        .collect()
}

#[derive(Debug, Clone)]
struct Job {
    tool: &'static str,
    sql: String,
    kinds: &'static [&'static str],
    dry_run: bool,
    confirm: bool,
    timeout_seconds: u64,
    progress_sql: Option<&'static str>,
    destructive_reason: Option<String>,
}

async fn run_maintenance(call: &Call, job: Job) -> Outcome {
    let Job {
        tool,
        sql,
        kinds,
        dry_run,
        confirm,
        timeout_seconds,
        progress_sql,
        destructive_reason,
    } = job;
    let mut classification = verify(&sql, kinds)?;
    if destructive_reason.is_some() {
        classification.destructive_reason = destructive_reason;
    }
    if dry_run {
        return dry_run_reply(&sql, &classification);
    }
    let decision = match call
        .context
        .gate
        .check(call, tool, &classification, confirm)?
    {
        Verdict::Proceed(decision) => decision,
        Verdict::Ask(result) => return Ok(ask(*result)),
    };
    let mut facts = facts_for(&classification);
    facts.decision = Some(decision);
    let timeout = Duration::from_secs(timeout_seconds.clamp(1, 86_400));
    let engine = call.engine().clone();
    let principal = call.principal.clone();
    let caps = call.caps(0);
    let statement = sql.clone();
    let outside = classification.runs_outside_transaction;
    let backend_pid = Arc::new(AtomicI32::new(0));
    let pid_report = Arc::clone(&backend_pid);
    let work = async move {
        engine
            .run_write_reporting_pid(
                &statement,
                caps,
                &principal,
                None,
                outside,
                Some(timeout),
                Some(&pid_report),
            )
            .await
    };
    let result = match (call.progress.clone(), progress_sql) {
        (Some(progress), Some(progress_sql)) => {
            progress.report(0.0, None, "starting".to_owned()).await;
            let mut work = std::pin::pin!(work);
            let mut ticks = 0f64;
            let outcome = loop {
                let poll = tokio::time::sleep(Duration::from_secs(1));
                tokio::select! {
                    outcome = &mut work => break outcome,
                    () = poll => {
                        ticks += 1.0;
                        let pid = backend_pid.load(Ordering::Relaxed);
                        let rows = if pid == 0 {
                            Vec::new()
                        } else {
                            call.engine().progress_rows(progress_sql, &[&pid]).await.unwrap_or_default()
                        };
                        let message = rows.first().map_or_else(
                            || "running".to_owned(),
                            |row| row.try_get::<_, String>(0).unwrap_or_else(|_| "running".to_owned()),
                        );
                        let done = rows.first().and_then(|row| row.try_get::<_, f64>(1).ok());
                        let total = rows.first().and_then(|row| row.try_get::<_, f64>(2).ok());
                        match (done, total) {
                            (Some(done), Some(total)) if total > 0.0 => {
                                progress.report(done, Some(total), message).await;
                            }
                            _ => progress.report(ticks, None, message).await,
                        }
                    }
                }
            };
            call.engine().drop_progress_lane().await;
            outcome
        }
        _ => work.await,
    };
    let result = result.map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
    let facts = facts.with_result(&result);
    let text = result.render_text();
    Ok(ToolOutput::structured(&result, text)?
        .with_facts(facts)
        .into())
}

const VACUUM_PROGRESS_SQL: &str = "SELECT phase::text || ' ' || relid::regclass::text, heap_blks_scanned::float8, heap_blks_total::float8 \
                                   FROM pg_catalog.pg_stat_progress_vacuum WHERE pid = $1";

pub fn vacuum(call: Call, args: VacuumArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if args.operation == VacuumOperation::Checkpoint {
            return run_maintenance(
                &call,
                Job {
                    tool: "pg_vacuum",
                    sql: "CHECKPOINT".to_owned(),
                    kinds: &["CheckPointStmt"],
                    dry_run: args.dry_run,
                    confirm: args.confirm,
                    timeout_seconds: args.timeout_seconds,
                    progress_sql: None,
                    destructive_reason: None,
                },
            )
            .await;
        }
        let tables = table_list(&call, &args.tables).await?;
        if tables.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "tables".to_owned(),
                detail: "the scoped schema holds no table to vacuum".to_owned(),
            }
            .into());
        }
        let mut options = Vec::new();
        if args.full {
            options.push("FULL".to_owned());
        }
        if args.freeze {
            options.push("FREEZE".to_owned());
        }
        if args.analyze {
            options.push("ANALYZE".to_owned());
        }
        if args.disable_page_skipping {
            options.push("DISABLE_PAGE_SKIPPING".to_owned());
        }
        if args.skip_locked {
            options.push("SKIP_LOCKED".to_owned());
        }
        match args.index_cleanup {
            IndexCleanup::Auto => {}
            IndexCleanup::On => options.push("INDEX_CLEANUP ON".to_owned()),
            IndexCleanup::Off => options.push("INDEX_CLEANUP OFF".to_owned()),
        }
        if let Some(truncate) = args.truncate.as_bool() {
            options.push(format!("TRUNCATE {}", if truncate { "ON" } else { "OFF" }));
        }
        if let Some(workers) = number("parallel", &args.parallel)? {
            options.push(format!("PARALLEL {}", workers.clamp(0, 1_024)));
        }
        if !args.buffer_usage_limit.trim().is_empty() {
            if !call.engine().features().supports_stat_io() {
                return Err(Error::ArgumentInvalid {
                    argument: "buffer_usage_limit".to_owned(),
                    detail: "BUFFER_USAGE_LIMIT needs PostgreSQL 16 or later".to_owned(),
                }
                .into());
            }
            let limit = args.buffer_usage_limit.trim();
            if !limit.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') {
                return Err(Error::ArgumentInvalid {
                    argument: "buffer_usage_limit".to_owned(),
                    detail: "use a size such as 256MB or 0".to_owned(),
                }
                .into());
            }
            options.push(format!("BUFFER_USAGE_LIMIT '{limit}'"));
        }
        let names: Vec<String> = tables.iter().map(QualifiedName::sql).collect();
        let sql = if options.is_empty() {
            format!("VACUUM {}", names.join(", "))
        } else {
            format!("VACUUM ({}) {}", options.join(", "), names.join(", "))
        };
        run_maintenance(
            &call,
            Job {
                tool: "pg_vacuum",
                sql,
                kinds: &["VacuumStmt"],
                dry_run: args.dry_run,
                confirm: args.confirm,
                timeout_seconds: args.timeout_seconds,
                progress_sql: Some(VACUUM_PROGRESS_SQL),
                destructive_reason: args
                    .full
                    .then(|| "VACUUM FULL rewrites the table under an exclusive lock".to_owned()),
            },
        )
        .await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeArgs {
    #[serde(default)]
    #[schemars(description = "Tables to analyze; empty means every table in the scoped schema.")]
    pub tables: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Columns to analyze, for a single table.")]
    pub columns: Vec<String>,
    #[serde(default)]
    pub skip_locked: bool,
    #[serde(default = "default_statement_timeout")]
    #[schemars(description = "Statement timeout in seconds for this run (default 600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn analyze(call: Call, args: AnalyzeArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let tables = table_list(&call, &args.tables).await?;
        if tables.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "tables".to_owned(),
                detail: "the scoped schema holds no table to analyze".to_owned(),
            }
            .into());
        }
        if !args.columns.is_empty() && tables.len() != 1 {
            return Err(Error::ArgumentInvalid {
                argument: "columns".to_owned(),
                detail: "columns apply to exactly one table".to_owned(),
            }
            .into());
        }
        let mut names: Vec<String> = tables.iter().map(QualifiedName::sql).collect();
        if !args.columns.is_empty()
            && let Some(first) = names.first_mut()
        {
            first.push_str(&format!(" ({})", ident_list("columns", &args.columns)?));
        }
        let options = if args.skip_locked {
            " (SKIP_LOCKED)"
        } else {
            ""
        };
        let sql = format!("ANALYZE{options} {}", names.join(", "));
        run_maintenance(
            &call,
            Job {
                tool: "pg_analyze",
                sql,
                kinds: &["VacuumStmt"],
                dry_run: args.dry_run,
                confirm: false,
                timeout_seconds: args.timeout_seconds,
                progress_sql: None,
                destructive_reason: None,
            },
        )
        .await
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReindexScope {
    #[default]
    Index,
    Table,
    Schema,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReindexArgs {
    #[serde(default)]
    pub target: ReindexScope,
    #[serde(default)]
    #[schemars(description = "Index or table name; empty for the scoped schema.")]
    pub name: String,
    #[serde(default)]
    pub concurrently: bool,
    #[serde(default = "default_statement_timeout")]
    #[schemars(description = "Statement timeout in seconds for this run (default 600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn reindex(call: Call, args: ReindexArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let concurrently = if args.concurrently {
            " CONCURRENTLY"
        } else {
            ""
        };
        let sql = match args.target {
            ReindexScope::Index => format!(
                "REINDEX INDEX{concurrently} {}",
                scoped_name(&call, "name", &args.name)?.sql()
            ),
            ReindexScope::Table => format!(
                "REINDEX TABLE{concurrently} {}",
                scoped_name(&call, "name", &args.name)?.sql()
            ),
            ReindexScope::Schema => format!(
                "REINDEX SCHEMA{concurrently} {}",
                quote_ident(&call.settings().schema.value)
            ),
        };
        run_maintenance(
            &call,
            Job {
                tool: "pg_reindex",
                sql,
                kinds: &["ReindexStmt"],
                dry_run: args.dry_run,
                confirm: false,
                timeout_seconds: args.timeout_seconds,
                progress_sql: None,
                destructive_reason: None,
            },
        )
        .await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefreshArgs {
    #[schemars(description = "The materialized view, optionally schema-qualified.")]
    pub view: String,
    #[serde(default)]
    pub concurrently: bool,
    #[serde(default = "default_with_data")]
    pub with_data: bool,
    #[serde(default = "default_statement_timeout")]
    #[schemars(description = "Statement timeout in seconds for this run (default 600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

const fn default_with_data() -> bool {
    true
}

pub fn refresh(call: Call, args: RefreshArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let view = scoped_name(&call, "view", &args.view)?;
        let sql = format!(
            "REFRESH MATERIALIZED VIEW{} {}{}",
            if args.concurrently {
                " CONCURRENTLY"
            } else {
                ""
            },
            view.sql(),
            if args.with_data {
                " WITH DATA"
            } else {
                " WITH NO DATA"
            }
        );
        run_maintenance(
            &call,
            Job {
                tool: "pg_refresh",
                sql,
                kinds: &["RefreshMatViewStmt"],
                dry_run: args.dry_run,
                confirm: false,
                timeout_seconds: args.timeout_seconds,
                progress_sql: None,
                destructive_reason: None,
            },
        )
        .await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VacuumNeed {
    pub table: String,
    pub live_rows: i64,
    pub dead_rows: i64,
    pub modified_since_analyze: i64,
    pub vacuum_threshold: i64,
    pub analyze_threshold: i64,
    pub needs_vacuum: bool,
    pub needs_analyze: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_vacuum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_autovacuum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_analyze: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_autoanalyze: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VacuumNeeds {
    pub rows: Vec<VacuumNeed>,
    pub row_count: usize,
    pub order: &'static str,
    pub notice: &'static str,
}

const VACUUM_NEEDS_SQL: &str = "WITH settings AS ( \
SELECT current_setting('autovacuum_vacuum_threshold')::float8 AS vt, current_setting('autovacuum_vacuum_scale_factor')::float8 AS vs, \
current_setting('autovacuum_analyze_threshold')::float8 AS at, current_setting('autovacuum_analyze_scale_factor')::float8 AS ascale \
), tables AS ( \
SELECT c.oid, c.relname::text AS name, c.reltuples::float8 AS reltuples, \
COALESCE((SELECT option_value::float8 FROM pg_catalog.pg_options_to_table(c.reloptions) WHERE option_name = 'autovacuum_vacuum_threshold'), s.vt) AS vt, \
COALESCE((SELECT option_value::float8 FROM pg_catalog.pg_options_to_table(c.reloptions) WHERE option_name = 'autovacuum_vacuum_scale_factor'), s.vs) AS vs, \
COALESCE((SELECT option_value::float8 FROM pg_catalog.pg_options_to_table(c.reloptions) WHERE option_name = 'autovacuum_analyze_threshold'), s.at) AS at, \
COALESCE((SELECT option_value::float8 FROM pg_catalog.pg_options_to_table(c.reloptions) WHERE option_name = 'autovacuum_analyze_scale_factor'), s.ascale) AS ascale \
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace CROSS JOIN settings s \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'm') \
) \
SELECT t.name, COALESCE(st.n_live_tup, 0)::int8, COALESCE(st.n_dead_tup, 0)::int8, COALESCE(st.n_mod_since_analyze, 0)::int8, \
(t.vt + t.vs * GREATEST(t.reltuples, 0))::int8, (t.at + t.ascale * GREATEST(t.reltuples, 0))::int8, \
COALESCE(st.n_dead_tup, 0) > (t.vt + t.vs * GREATEST(t.reltuples, 0)), \
COALESCE(st.n_mod_since_analyze, 0) > (t.at + t.ascale * GREATEST(t.reltuples, 0)), \
st.last_vacuum::text, st.last_autovacuum::text, st.last_analyze::text, st.last_autoanalyze::text \
FROM tables t LEFT JOIN pg_catalog.pg_stat_user_tables st ON st.relid = t.oid \
ORDER BY COALESCE(st.n_dead_tup, 0) DESC, t.name";

pub fn vacuum_needs(call: Call, _args: super::health::NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        let rows = call
            .engine()
            .catalog_rows(VACUUM_NEEDS_SQL, &[&scoped])
            .await?;
        let mut needs = Vec::new();
        for row in &rows {
            needs.push(VacuumNeed {
                table: catalog::read_column(row, 0)?,
                live_rows: catalog::read_column(row, 1)?,
                dead_rows: catalog::read_column(row, 2)?,
                modified_since_analyze: catalog::read_column(row, 3)?,
                vacuum_threshold: catalog::read_column(row, 4)?,
                analyze_threshold: catalog::read_column(row, 5)?,
                needs_vacuum: catalog::read_column(row, 6)?,
                needs_analyze: catalog::read_column(row, 7)?,
                last_vacuum: catalog::read_column(row, 8)?,
                last_autovacuum: catalog::read_column(row, 9)?,
                last_analyze: catalog::read_column(row, 10)?,
                last_autoanalyze: catalog::read_column(row, 11)?,
            });
        }
        let text = text_rows(
            &[
                ("table", "text"),
                ("live_rows", "int8"),
                ("dead_rows", "int8"),
                ("vacuum_threshold", "int8"),
                ("needs_vacuum", "bool"),
                ("needs_analyze", "bool"),
                ("last_autovacuum", "text"),
            ],
            needs
                .iter()
                .map(|need| {
                    vec![
                        Some(need.table.clone()),
                        Some(need.live_rows.to_string()),
                        Some(need.dead_rows.to_string()),
                        Some(need.vacuum_threshold.to_string()),
                        Some(need.needs_vacuum.to_string()),
                        Some(need.needs_analyze.to_string()),
                        need.last_autovacuum.clone(),
                    ]
                })
                .collect(),
            None,
            None,
        );
        let result = VacuumNeeds {
            row_count: needs.len(),
            rows: needs,
            order: "dead_rows desc, table",
            notice: UNTRUSTED_NOTICE,
        };
        Ok(ToolOutput::structured(&result, text)?
            .with_facts(AuditFacts {
                operation: Some("vacuum_needs".to_owned()),
                row_count: Some(result.row_count as u64),
                ..AuditFacts::default()
            })
            .into())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendOperation {
    Cancel,
    Terminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BackendArgs {
    pub operation: BackendOperation,
    #[schemars(description = "The backend process id from pg_activity.")]
    pub pid: i32,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
}

pub fn backend(call: Call, args: BackendArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if args.pid <= 0 {
            return Err(Error::ArgumentInvalid {
                argument: "pid".to_owned(),
                detail: "a positive backend pid is required".to_owned(),
            }
            .into());
        }
        let function = match args.operation {
            BackendOperation::Cancel => "pg_cancel_backend",
            BackendOperation::Terminate => "pg_terminate_backend",
        };
        let sql = format!("SELECT pg_catalog.{function}({}) AS signalled", args.pid);
        let mut classification = verify(&sql, &["SelectStmt"])?;
        classification.refusals.clear();
        let settings = call.settings();
        let pooled = call.engine().info().await.pooled;
        let scope = SchemaScope {
            schema: &settings.schema.value,
            require_qualified_names: pooled,
        };
        classify::authorize(&classification, settings.mode.value, &scope)
            .map_err(|error| ToolFailure::from(error).with_facts(facts_for(&classification)))?;
        if args.operation == BackendOperation::Terminate
            && classification.destructive_reason.is_none()
        {
            classification.destructive_reason = Some(format!(
                "terminating backend {} drops its session",
                args.pid
            ));
        }
        if args.dry_run {
            return dry_run_reply(&sql, &classification);
        }
        let decision =
            match call
                .context
                .gate
                .check(&call, "pg_backend", &classification, args.confirm)?
            {
                Verdict::Proceed(decision) => decision,
                Verdict::Ask(result) => return Ok(ask(*result)),
            };
        let mut facts = facts_for(&classification);
        facts.decision = Some(decision);
        let result: ResultSet = call
            .engine()
            .run_write(&sql, call.caps(1), &call.principal, None, false)
            .await
            .map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
        let facts = facts.with_result(&result);
        let text = result.render_text();
        Ok(ToolOutput::structured(&result, text)?
            .with_facts(facts)
            .into())
    })
}

pub fn routes() -> Result<Vec<Route>> {
    Ok(vec![
        route::<VacuumArgs, ResultSet, _>(&tool_specs::PG_VACUUM, VACUUM_DESCRIPTION, vacuum)?,
        route::<AnalyzeArgs, ResultSet, _>(&tool_specs::PG_ANALYZE, ANALYZE_DESCRIPTION, analyze)?,
        route::<ReindexArgs, ResultSet, _>(&tool_specs::PG_REINDEX, REINDEX_DESCRIPTION, reindex)?,
        route::<RefreshArgs, ResultSet, _>(&tool_specs::PG_REFRESH, REFRESH_DESCRIPTION, refresh)?,
        route::<super::health::NoArgs, VacuumNeeds, _>(
            &tool_specs::PG_VACUUM_NEEDS,
            VACUUM_NEEDS_DESCRIPTION,
            vacuum_needs,
        )?,
        route::<BackendArgs, ResultSet, _>(&tool_specs::PG_BACKEND, BACKEND_DESCRIPTION, backend)?,
    ])
}
