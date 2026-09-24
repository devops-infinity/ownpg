use std::collections::BTreeMap;

use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::catalog;
use super::monitoring;
use super::{AuditFacts, Call, Outcome, Route, ToolOutput, route};
use crate::config::describe::{SettingLine, describe};
use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::shape::UNTRUSTED_NOTICE;
use crate::tool_specs;

const HEALTH_DESCRIPTION: &str = "Summarize the health of the connected server and database: connection use against max_connections, the buffer cache hit ratio, the transaction ID age of the database against autovacuum_freeze_max_age, the longest running transaction, sessions idle in a transaction, invalid and unused indexes in the scoped schema, estimated bloat in the scoped schema, replication lag and inactive slots, and the database size. Each check carries a status of ok, warning, or critical with the measured value and a short explanation. Checks are listed in that fixed order.";

const DOCTOR_DESCRIPTION: &str = "Report how this server is connected and configured: the target and transport, TLS state, server version, connected role and its attributes, database, schema, access mode, loaded tool groups, effective limits, the audit log path and a warning when an entry could not be written, open cursor handles, the feature map for this server version, and the absolute path and version of each PostgreSQL host program (pg_dump, pg_dumpall, pg_restore, pg_basebackup, pg_upgrade) or that it was not found. Secrets are never included.";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Warning,
    Critical,
}

impl Status {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub value: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct HealthReport {
    pub database: String,
    pub server_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_warning: Option<String>,
    pub status: Status,
    pub checks: Vec<Check>,
    pub notice: &'static str,
}

fn worst(checks: &[Check]) -> Status {
    let mut status = Status::Ok;
    for check in checks {
        status = match (status, check.status) {
            (_, Status::Critical) | (Status::Critical, _) => Status::Critical,
            (_, Status::Warning) | (Status::Warning, _) => Status::Warning,
            (Status::Ok, Status::Ok) => Status::Ok,
        };
    }
    status
}

pub async fn public_schema_grants_create(engine: &Engine) -> Result<bool> {
    let rows = engine
        .catalog_rows(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n \
             CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) a \
             WHERE n.nspname = 'public' AND a.grantee = 0 AND a.privilege_type = 'CREATE')",
            &[],
        )
        .await?;
    match rows.first() {
        Some(row) => catalog::read_column(row, 0),
        None => Ok(false),
    }
}

pub async fn health_report(engine: &Engine) -> Result<HealthReport> {
    let info = engine.info().await;
    let scoped = engine.settings().schema.value.clone();
    let mut checks = Vec::new();

    let rows = engine
        .catalog_rows(
            "SELECT (SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE backend_type = 'client backend')::int8, \
             pg_catalog.current_setting('max_connections')::int8, \
             pg_catalog.current_setting('superuser_reserved_connections')::int8",
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let used: i64 = catalog::read_column(row, 0)?;
        let max: i64 = catalog::read_column(row, 1)?;
        let reserved: i64 = catalog::read_column(row, 2)?;
        let usable = (max - reserved).max(1);
        let ratio = used as f64 / usable as f64;
        let status = if ratio >= 0.95 {
            Status::Critical
        } else if ratio >= 0.80 {
            Status::Warning
        } else {
            Status::Ok
        };
        checks.push(Check {
            name: "connections".to_owned(),
            status,
            value: format!("{used} of {usable}"),
            detail: format!(
                "{used} client backends against max_connections {max} minus {reserved} reserved"
            ),
        });
    }

    let rows = engine
        .catalog_rows(
            "SELECT COALESCE(sum(blks_hit), 0)::int8, COALESCE(sum(blks_read), 0)::int8 \
             FROM pg_catalog.pg_stat_database WHERE datname = pg_catalog.current_database()",
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let hit: i64 = catalog::read_column(row, 0)?;
        let read: i64 = catalog::read_column(row, 1)?;
        let total = hit + read;
        let ratio = if total == 0 {
            1.0
        } else {
            hit as f64 / total as f64
        };
        let status = if total < 10_000 {
            Status::Ok
        } else if ratio < 0.80 {
            Status::Critical
        } else if ratio < 0.90 {
            Status::Warning
        } else {
            Status::Ok
        };
        checks.push(Check {
            name: "cache_hit_ratio".to_owned(),
            status,
            value: format!("{:.3}", ratio),
            detail: format!("{hit} buffer hits and {read} disk reads since the statistics reset"),
        });
    }

    let rows = engine
        .catalog_rows(
            "SELECT pg_catalog.age(d.datfrozenxid)::int8, pg_catalog.current_setting('autovacuum_freeze_max_age')::int8, \
             (SELECT max(pg_catalog.age(datfrozenxid)) FROM pg_catalog.pg_database)::int8 \
             FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()",
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let age: i64 = catalog::read_column(row, 0)?;
        let freeze_max: i64 = catalog::read_column(row, 1)?;
        let cluster_max: i64 = catalog::read_column(row, 2)?;
        let status = if cluster_max >= 1_500_000_000 {
            Status::Critical
        } else if age >= freeze_max || cluster_max >= 1_000_000_000 {
            Status::Warning
        } else {
            Status::Ok
        };
        checks.push(Check {
            name: "transaction_id_age".to_owned(),
            status,
            value: age.to_string(),
            detail: format!(
                "this database's oldest transaction ID is {age} old; autovacuum freezes at {freeze_max}; the cluster maximum is {cluster_max} and wraparound protection stops writes near 2 billion"
            ),
        });
    }

    let rows = engine
        .catalog_rows(
            "SELECT COALESCE(max(EXTRACT(EPOCH FROM pg_catalog.now() - xact_start)), 0)::float8, \
             (SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE state = 'idle in transaction')::int8 \
             FROM pg_catalog.pg_stat_activity WHERE backend_type = 'client backend' AND xact_start IS NOT NULL",
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let longest: f64 = catalog::read_column(row, 0)?;
        let idle: i64 = catalog::read_column(row, 1)?;
        let status = if longest >= 3_600.0 {
            Status::Critical
        } else if longest >= 300.0 {
            Status::Warning
        } else {
            Status::Ok
        };
        checks.push(Check {
            name: "longest_transaction".to_owned(),
            status,
            value: format!("{longest:.0} s"),
            detail: "the longest open transaction across client backends, including this one"
                .to_owned(),
        });
        checks.push(Check {
            name: "idle_in_transaction".to_owned(),
            status: if idle > 0 {
                Status::Warning
            } else {
                Status::Ok
            },
            value: idle.to_string(),
            detail: "sessions holding a transaction open while idle block vacuum and hold locks"
                .to_owned(),
        });
    }

    let rows = engine
        .catalog_rows(
            "SELECT count(*)::int8 FROM pg_catalog.pg_index i \
             JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE NOT i.indisvalid AND n.nspname = $1",
            &[&scoped],
        )
        .await?;
    if let Some(row) = rows.first() {
        let invalid: i64 = catalog::read_column(row, 0)?;
        checks.push(Check {
            name: "invalid_indexes".to_owned(),
            status: if invalid > 0 {
                Status::Warning
            } else {
                Status::Ok
            },
            value: invalid.to_string(),
            detail: format!(
                "indexes in schema {scoped} left invalid by a failed concurrent build; reindex or drop them"
            ),
        });
    }

    let rows = engine
        .catalog_rows(
            &format!(
                "SELECT count(*)::int8, COALESCE(sum(size_bytes), 0)::int8 FROM ({}) AS findings WHERE problem = 'unused'",
                monitoring::indexes_health_sql(&scoped, engine.features())
            ),
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let unused: i64 = catalog::read_column(row, 0)?;
        let bytes: i64 = catalog::read_column(row, 1)?;
        checks.push(Check {
            name: "unused_indexes".to_owned(),
            status: if unused > 0 && bytes >= 64 * 1024 * 1024 {
                Status::Warning
            } else {
                Status::Ok
            },
            value: format!("{unused} using {bytes} bytes"),
            detail: format!(
                "indexes in schema {scoped} with zero scans since the statistics reset that are neither unique nor a primary key; pg_indexes_health lists them"
            ),
        });
    }

    let rows = engine
        .catalog_rows(
            &format!(
                "SELECT COALESCE(sum(wasted_bytes) FILTER (WHERE NOT is_na), 0)::int8, COALESCE(sum(real_bytes) FILTER (WHERE NOT is_na), 0)::int8, COALESCE(bool_or(is_na), false) FROM ({}) AS bloat",
                monitoring::bloat_sql(&scoped)
            ),
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let wasted: i64 = catalog::read_column(row, 0)?;
        let real: i64 = catalog::read_column(row, 1)?;
        let uncertain: bool = catalog::read_column(row, 2)?;
        let ratio = if real == 0 {
            0.0
        } else {
            wasted as f64 / real as f64
        };
        let status = if ratio >= 0.50 && wasted >= 1024 * 1024 * 1024 {
            Status::Critical
        } else if ratio >= 0.20 && wasted >= 64 * 1024 * 1024 {
            Status::Warning
        } else {
            Status::Ok
        };
        checks.push(Check {
            name: "bloat".to_owned(),
            status,
            value: format!("{wasted} of {real} bytes"),
            detail: format!(
                "estimated wasted space across tables and B-tree indexes in schema {scoped} from pg_class and pg_stats{}; pg_bloat lists each relation",
                if uncertain {
                    " (some relations could not be estimated and are left out of the totals)"
                } else {
                    ""
                }
            ),
        });
    }

    let rows = engine
        .catalog_rows(
            "SELECT pg_catalog.pg_is_in_recovery(), \
             (CASE WHEN pg_catalog.pg_is_in_recovery() THEN COALESCE(pg_catalog.pg_wal_lsn_diff(pg_catalog.pg_last_wal_receive_lsn(), pg_catalog.pg_last_wal_replay_lsn()), 0) ELSE 0 END)::int8, \
             (CASE WHEN pg_catalog.pg_is_in_recovery() THEN COALESCE(EXTRACT(EPOCH FROM pg_catalog.now() - pg_catalog.pg_last_xact_replay_timestamp()), 0) ELSE 0 END)::float8, \
             (SELECT count(*) FROM pg_catalog.pg_stat_replication)::int8, \
             (SELECT COALESCE(max(pg_catalog.pg_wal_lsn_diff(sent_lsn, replay_lsn)), 0) FROM pg_catalog.pg_stat_replication)::int8, \
             (SELECT count(*) FROM pg_catalog.pg_replication_slots WHERE NOT active)::int8, \
             (SELECT COALESCE(max(CASE WHEN pg_catalog.pg_is_in_recovery() THEN 0 ELSE pg_catalog.pg_wal_lsn_diff(pg_catalog.pg_current_wal_lsn(), restart_lsn) END), 0) \
              FROM pg_catalog.pg_replication_slots WHERE NOT active)::int8",
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let in_recovery: bool = catalog::read_column(row, 0)?;
        let replay_lag: i64 = catalog::read_column(row, 1)?;
        let replay_age: f64 = catalog::read_column(row, 2)?;
        let standbys: i64 = catalog::read_column(row, 3)?;
        let standby_lag: i64 = catalog::read_column(row, 4)?;
        let inactive_slots: i64 = catalog::read_column(row, 5)?;
        let retained: i64 = catalog::read_column(row, 6)?;
        const WARN_BYTES: i64 = 64 * 1024 * 1024;
        const CRITICAL_BYTES: i64 = 1024 * 1024 * 1024;
        let (status, value, detail) = if in_recovery {
            let status = if replay_lag >= CRITICAL_BYTES || replay_age >= 300.0 {
                Status::Critical
            } else if replay_lag >= WARN_BYTES || replay_age >= 60.0 {
                Status::Warning
            } else {
                Status::Ok
            };
            (
                status,
                format!("standby, {replay_lag} bytes behind"),
                format!(
                    "this server is a standby; {replay_lag} bytes received but not replayed, last replayed transaction {replay_age:.0} s ago"
                ),
            )
        } else {
            let status = if standby_lag >= CRITICAL_BYTES || retained >= CRITICAL_BYTES {
                Status::Critical
            } else if inactive_slots > 0 || standby_lag >= WARN_BYTES {
                Status::Warning
            } else {
                Status::Ok
            };
            (
                status,
                format!("primary, {standbys} standbys"),
                format!(
                    "{standbys} connected standbys with at most {standby_lag} bytes sent but not replayed; {inactive_slots} inactive replication slots holding {retained} bytes of WAL; pg_replication lists each one"
                ),
            )
        };
        checks.push(Check {
            name: "replication".to_owned(),
            status,
            value,
            detail,
        });
    }

    let rows = engine
        .catalog_rows(
            "SELECT pg_catalog.pg_database_size(pg_catalog.current_database())::int8",
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let size: i64 = catalog::read_column(row, 0)?;
        checks.push(Check {
            name: "database_size".to_owned(),
            status: Status::Ok,
            value: format!("{size} bytes"),
            detail: "total on-disk size of the connected database".to_owned(),
        });
    }

    Ok(HealthReport {
        server_warning: engine.features().version_warning(&info.server_version),
        database: info.database,
        server_version: info.server_version,
        status: worst(&checks),
        checks,
        notice: UNTRUSTED_NOTICE,
    })
}

pub fn health(call: Call, _args: NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let context = call.context.clone();
        let report = health_report(&context.engine).await?;
        let mut text = format!(
            "{}\nhealth of {} on PostgreSQL {}: {}\n",
            UNTRUSTED_NOTICE,
            report.database,
            report.server_version,
            report.status.as_str()
        );
        for check in &report.checks {
            text.push_str(&format!(
                "  {} [{}] {}: {}\n",
                check.name,
                check.status.as_str(),
                check.value,
                check.detail
            ));
        }
        Ok(ToolOutput::structured(&report, text)?
            .with_facts(AuditFacts {
                operation: Some("health".to_owned()),
                row_count: Some(report.checks.len() as u64),
                ..AuditFacts::default()
            })
            .into())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct AttemptLine {
    pub target: String,
    pub user: String,
    pub outcome: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DoctorReport {
    pub target: String,
    pub via: String,
    pub tls: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_warning: Option<String>,
    pub server_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_warning: Option<String>,
    pub role: String,
    pub role_attributes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_warning: Option<String>,
    pub database: String,
    pub schema: String,
    pub search_path: String,
    pub pooled: bool,
    pub mode: String,
    pub loaded_groups: Vec<String>,
    pub tools: Vec<String>,
    pub settings: Vec<SettingLine>,
    pub attempts: Vec<AttemptLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_warning: Option<String>,
    pub open_cursors: usize,
    pub features: BTreeMap<String, bool>,
    pub host_programs: Vec<super::host::HostProgram>,
    pub version: String,
}

pub async fn doctor_report(
    engine: &Engine,
    audit_path: Option<&std::path::Path>,
    audit_warning: Option<String>,
) -> Result<DoctorReport> {
    let info = engine.info().await;
    let settings = engine.settings();
    let role = engine.role().await?;
    let mut attributes = Vec::new();
    if role.superuser {
        attributes.push("superuser".to_owned());
    }
    if role.bypass_rls {
        attributes.push("bypassrls".to_owned());
    }
    if role.create_db {
        attributes.push("createdb".to_owned());
    }
    if role.create_role {
        attributes.push("createrole".to_owned());
    }
    if role.replication {
        attributes.push("replication".to_owned());
    }
    if role.rds_superuser {
        attributes.push("rds_superuser".to_owned());
    }
    for membership in &role.memberships {
        attributes.push(format!("member of {membership}"));
    }
    let setting_lines = describe(settings);
    let attempts = info
        .attempts
        .iter()
        .map(|attempt| AttemptLine {
            target: attempt.target.clone(),
            user: attempt.user.clone(),
            outcome: match &attempt.outcome {
                Ok(()) => "connected".to_owned(),
                Err(reason) => format!("failed: {reason}"),
            },
        })
        .collect();
    let tls = match (&info.tls, info.tls_used) {
        (None, _) => "not applicable (socket)".to_owned(),
        (Some(plan), Some(true)) => format!("encrypted, {}", plan.describe()),
        (Some(plan), _) => format!("not encrypted, {}", plan.describe()),
    };
    Ok(DoctorReport {
        target: info.target.clone(),
        via: info.via.as_str().to_owned(),
        tls,
        tls_warning: info.tls_warning(),
        server_version: info.server_version.clone(),
        server_warning: engine.features().version_warning(&info.server_version),
        role: role.name.clone(),
        role_attributes: attributes,
        role_warning: role.warning(),
        database: info.database.clone(),
        schema: settings.schema.value.clone(),
        search_path: info.search_path.clone(),
        pooled: info.pooled,
        mode: settings.mode.value.to_string(),
        loaded_groups: settings
            .loaded_groups()
            .iter()
            .map(ToString::to_string)
            .collect(),
        tools: tool_specs::loaded(settings)
            .iter()
            .map(|tool| tool.name.to_owned())
            .collect(),
        settings: setting_lines,
        attempts,
        audit_path: audit_path.map(|path| path.display().to_string()),
        audit_warning,
        open_cursors: engine.open_cursors().await.len(),
        features: engine
            .features()
            .as_map()
            .into_iter()
            .map(|(name, enabled)| (name.to_owned(), enabled))
            .collect(),
        host_programs: super::host::program_report(settings).await,
        version: crate::VERSION.to_owned(),
    })
}

fn hide_local_paths(report: &mut DoctorReport) {
    let hidden = crate::config::describe::MASKED;
    for line in &mut report.settings {
        if std::path::Path::new(&line.value).is_absolute() {
            hidden.clone_into(&mut line.value);
        }
    }
    if report.audit_path.is_some() {
        report.audit_path = Some(hidden.to_owned());
    }
    for program in &mut report.host_programs {
        if program.path.is_some() {
            program.path = Some(hidden.to_owned());
        }
    }
}

pub fn doctor(call: Call, _args: NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let context = call.context.clone();
        let mut report = doctor_report(
            &context.engine,
            context.audit.path(),
            context.audit.warning(),
        )
        .await?;
        if context.transport == crate::audit::Transport::Http {
            hide_local_paths(&mut report);
        }
        let text = render_doctor(&report);
        Ok(ToolOutput::structured(&report, text)?
            .with_facts(AuditFacts {
                operation: Some("doctor".to_owned()),
                ..AuditFacts::default()
            })
            .into())
    })
}

#[must_use]
pub fn render_doctor(report: &DoctorReport) -> String {
    let mut text = String::new();
    text.push_str(&format!("OwnPG {}\n", report.version));
    text.push_str(&format!("target: {} via {}\n", report.target, report.via));
    text.push_str(&format!("tls: {}\n", report.tls));
    if let Some(warning) = &report.tls_warning {
        text.push_str(&format!("warning: {warning}\n"));
    }
    text.push_str(&format!("server: PostgreSQL {}\n", report.server_version));
    if let Some(warning) = &report.server_warning {
        text.push_str(&format!("warning: {warning}\n"));
    }
    text.push_str(&format!(
        "role: {}{}\n",
        report.role,
        if report.role_attributes.is_empty() {
            String::new()
        } else {
            format!(" ({})", report.role_attributes.join(", "))
        }
    ));
    if let Some(warning) = &report.role_warning {
        text.push_str(&format!("warning: {warning}\n"));
    }
    text.push_str(&format!(
        "database: {}, schema: {}, search_path: {}{}\n",
        report.database,
        report.schema,
        report.search_path,
        if report.pooled { " (pooled)" } else { "" }
    ));
    text.push_str(&format!(
        "mode: {}, groups: {}\n",
        report.mode,
        if report.loaded_groups.is_empty() {
            "default only".to_owned()
        } else {
            report.loaded_groups.join(", ")
        }
    ));
    text.push_str(&format!("tools: {}\n", report.tools.join(", ")));
    for setting in &report.settings {
        text.push_str(&format!(
            "setting {} = {} (from {})\n",
            setting.name, setting.value, setting.origin
        ));
    }
    for attempt in &report.attempts {
        text.push_str(&format!(
            "attempt {} as {}: {}\n",
            attempt.target, attempt.user, attempt.outcome
        ));
    }
    match &report.audit_path {
        Some(path) => text.push_str(&format!("audit: {path}\n")),
        None => text.push_str("audit: off\n"),
    }
    if let Some(warning) = &report.audit_warning {
        text.push_str(&format!("warning: {warning}\n"));
    }
    text.push_str(&format!("open cursors: {}\n", report.open_cursors));
    let features: Vec<String> = report
        .features
        .iter()
        .map(|(name, enabled)| format!("{name}={enabled}"))
        .collect();
    text.push_str(&format!("features: {}\n", features.join(", ")));
    for program in &report.host_programs {
        match (&program.path, &program.version) {
            (Some(path), Some(version)) => {
                text.push_str(&format!("{}: {path} ({version})\n", program.name));
            }
            (Some(path), None) => {
                text.push_str(&format!(
                    "{}: {path} (the version could not be read)\n",
                    program.name
                ));
            }
            (None, _) => text.push_str(&format!("{}: not found\n", program.name)),
        }
    }
    text
}

pub fn routes() -> Result<Vec<Route>, Error> {
    Ok(vec![
        route::<NoArgs, HealthReport, _>(&tool_specs::PG_HEALTH, HEALTH_DESCRIPTION, health)?,
        route::<NoArgs, DoctorReport, _>(&tool_specs::PG_DOCTOR, DOCTOR_DESCRIPTION, doctor)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: Status) -> Check {
        Check {
            name: "x".to_owned(),
            status,
            value: String::new(),
            detail: String::new(),
        }
    }

    fn report(audit_warning: Option<String>) -> DoctorReport {
        DoctorReport {
            target: "127.0.0.1:5432".to_owned(),
            via: "tcp".to_owned(),
            tls: "not applicable (socket)".to_owned(),
            tls_warning: None,
            server_version: "18.6".to_owned(),
            server_warning: None,
            role: "app".to_owned(),
            role_attributes: Vec::new(),
            role_warning: None,
            database: "app".to_owned(),
            schema: "app".to_owned(),
            search_path: "app".to_owned(),
            pooled: false,
            mode: "read-only".to_owned(),
            loaded_groups: Vec::new(),
            tools: Vec::new(),
            settings: Vec::new(),
            attempts: Vec::new(),
            audit_path: Some("/tmp/audit.jsonl".to_owned()),
            audit_warning,
            open_cursors: 0,
            features: BTreeMap::new(),
            host_programs: Vec::new(),
            version: "0.1.0".to_owned(),
        }
    }

    #[test]
    fn remote_callers_see_that_a_path_is_set_but_not_where_it_is() {
        let mut shown = report(None);
        shown.settings = vec![
            crate::config::describe::SettingLine {
                name: "config_file".to_owned(),
                value: "/home/alice/.config/ownpg/profiles.toml".to_owned(),
                origin: "preset".to_owned(),
            },
            crate::config::describe::SettingLine {
                name: "mode".to_owned(),
                value: "read-only".to_owned(),
                origin: "flag".to_owned(),
            },
        ];
        shown.host_programs = vec![super::super::host::HostProgram {
            name: "pg_dump".to_owned(),
            path: Some("/opt/pg/bin/pg_dump".to_owned()),
            version: Some("18.6".to_owned()),
        }];
        hide_local_paths(&mut shown);
        assert_eq!(shown.settings[0].value, "set");
        assert_eq!(shown.settings[1].value, "read-only");
        assert_eq!(shown.audit_path.as_deref(), Some("set"));
        assert_eq!(shown.host_programs[0].path.as_deref(), Some("set"));
        assert_eq!(shown.host_programs[0].version.as_deref(), Some("18.6"));
    }

    #[test]
    fn a_degraded_audit_log_is_reported_next_to_the_audit_path() {
        let healthy = render_doctor(&report(None));
        assert!(healthy.contains("audit: /tmp/audit.jsonl\n"), "{healthy}");
        assert!(!healthy.contains("warning:"), "{healthy}");
        let json = serde_json::to_value(report(None)).unwrap();
        assert!(json.get("audit_warning").is_none(), "{json}");

        let degraded = render_doctor(&report(Some(
            "the audit log is degraded: 2 entries could not be written".to_owned(),
        )));
        assert!(
            degraded.contains(
                "audit: /tmp/audit.jsonl\nwarning: the audit log is degraded: 2 entries could not be written\n"
            ),
            "{degraded}"
        );
    }

    #[test]
    fn the_overall_status_is_the_worst_check() {
        assert_eq!(worst(&[]), Status::Ok);
        assert_eq!(
            worst(&[check(Status::Ok), check(Status::Warning)]),
            Status::Warning
        );
        assert_eq!(
            worst(&[
                check(Status::Warning),
                check(Status::Critical),
                check(Status::Ok)
            ]),
            Status::Critical
        );
    }
}
