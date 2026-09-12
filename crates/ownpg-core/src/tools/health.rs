use std::collections::BTreeMap;

use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::catalog;
use super::{AuditFacts, Context, Outcome, Route, ToolOutput, route};
use crate::config::Origin;
use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::groups;
use crate::shape::UNTRUSTED_NOTICE;

const HEALTH_DESCRIPTION: &str = "Summarize the health of the connected server and database: connection use against max_connections, the buffer cache hit ratio, the transaction ID age of the database against autovacuum_freeze_max_age, the longest running transaction, sessions idle in a transaction, invalid indexes in the scoped schema, and the database size. Each check carries a status of ok, warning, or critical with the measured value and a short explanation.";

const DOCTOR_DESCRIPTION: &str = "Report how this server is connected and configured: the target and transport, TLS state, server version, connected role and its attributes, database, schema, access mode, loaded tool groups, effective limits, audit log path, open cursor handles, and the feature map for this server version. Secrets are never included.";

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
        let used: i64 = catalog::get(row, 0)?;
        let max: i64 = catalog::get(row, 1)?;
        let reserved: i64 = catalog::get(row, 2)?;
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
        let hit: i64 = catalog::get(row, 0)?;
        let read: i64 = catalog::get(row, 1)?;
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
        let age: i64 = catalog::get(row, 0)?;
        let freeze_max: i64 = catalog::get(row, 1)?;
        let cluster_max: i64 = catalog::get(row, 2)?;
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
        let longest: f64 = catalog::get(row, 0)?;
        let idle: i64 = catalog::get(row, 1)?;
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
        let invalid: i64 = catalog::get(row, 0)?;
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
            "SELECT pg_catalog.pg_database_size(pg_catalog.current_database())::int8",
            &[],
        )
        .await?;
    if let Some(row) = rows.first() {
        let size: i64 = catalog::get(row, 0)?;
        checks.push(Check {
            name: "database_size".to_owned(),
            status: Status::Ok,
            value: format!("{size} bytes"),
            detail: "total on-disk size of the connected database".to_owned(),
        });
    }

    Ok(HealthReport {
        database: info.database,
        server_version: info.server_version,
        status: worst(&checks),
        checks,
        notice: UNTRUSTED_NOTICE,
    })
}

pub fn health(context: Context, _args: NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
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
        Ok(
            ToolOutput::structured(&report, text)?.with_facts(AuditFacts {
                operation: Some("health".to_owned()),
                row_count: Some(report.checks.len() as u64),
                ..AuditFacts::default()
            }),
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SettingLine {
    pub name: String,
    pub value: String,
    pub origin: String,
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
    pub open_cursors: usize,
    pub features: BTreeMap<String, bool>,
    pub version: String,
}

fn origin_name(origin: Origin) -> String {
    serde_json::to_value(origin)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "preset".to_owned())
}

pub async fn doctor_report(
    engine: &Engine,
    audit_path: Option<&std::path::Path>,
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
    let limits = &settings.limits;
    let setting_lines = vec![
        SettingLine {
            name: "mode".to_owned(),
            value: settings.mode.value.to_string(),
            origin: origin_name(settings.mode.origin),
        },
        SettingLine {
            name: "database".to_owned(),
            value: settings.database.value.clone(),
            origin: origin_name(settings.database.origin),
        },
        SettingLine {
            name: "schema".to_owned(),
            value: settings.schema.value.clone(),
            origin: origin_name(settings.schema.origin),
        },
        SettingLine {
            name: "strict_role".to_owned(),
            value: settings.strict_role.value.to_string(),
            origin: origin_name(settings.strict_role.origin),
        },
        SettingLine {
            name: "statement_timeout".to_owned(),
            value: format!("{} ms", limits.statement_timeout.value.as_millis()),
            origin: origin_name(limits.statement_timeout.origin),
        },
        SettingLine {
            name: "lock_timeout".to_owned(),
            value: format!("{} ms", limits.lock_timeout.value.as_millis()),
            origin: origin_name(limits.lock_timeout.origin),
        },
        SettingLine {
            name: "transaction_timeout".to_owned(),
            value: format!("{} ms", limits.transaction_timeout.value.as_millis()),
            origin: origin_name(limits.transaction_timeout.origin),
        },
        SettingLine {
            name: "handle_expiry".to_owned(),
            value: format!("{} s", limits.handle_expiry.value.as_secs()),
            origin: origin_name(limits.handle_expiry.origin),
        },
        SettingLine {
            name: "row_cap".to_owned(),
            value: limits.row_cap.value.to_string(),
            origin: origin_name(limits.row_cap.origin),
        },
        SettingLine {
            name: "byte_cap".to_owned(),
            value: limits.byte_cap.value.to_string(),
            origin: origin_name(limits.byte_cap.origin),
        },
        SettingLine {
            name: "audit".to_owned(),
            value: settings.audit.enabled.value.to_string(),
            origin: origin_name(settings.audit.enabled.origin),
        },
    ];
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
        tools: groups::loaded(settings)
            .iter()
            .map(|tool| tool.name.to_owned())
            .collect(),
        settings: setting_lines,
        attempts,
        audit_path: audit_path.map(|path| path.display().to_string()),
        open_cursors: engine.open_cursors().await.len(),
        features: engine
            .features()
            .as_map()
            .into_iter()
            .map(|(name, enabled)| (name.to_owned(), enabled))
            .collect(),
        version: crate::VERSION.to_owned(),
    })
}

pub fn doctor(context: Context, _args: NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let report = doctor_report(&context.engine, context.audit_path.as_deref()).await?;
        let text = render_doctor(&report);
        Ok(
            ToolOutput::structured(&report, text)?.with_facts(AuditFacts {
                operation: Some("doctor".to_owned()),
                ..AuditFacts::default()
            }),
        )
    })
}

#[must_use]
pub fn render_doctor(report: &DoctorReport) -> String {
    let mut text = String::new();
    text.push_str(&format!("ownpg {}\n", report.version));
    text.push_str(&format!("target: {} via {}\n", report.target, report.via));
    text.push_str(&format!("tls: {}\n", report.tls));
    if let Some(warning) = &report.tls_warning {
        text.push_str(&format!("warning: {warning}\n"));
    }
    text.push_str(&format!("server: PostgreSQL {}\n", report.server_version));
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
    text.push_str(&format!("open cursors: {}\n", report.open_cursors));
    let features: Vec<String> = report
        .features
        .iter()
        .map(|(name, enabled)| format!("{name}={enabled}"))
        .collect();
    text.push_str(&format!("features: {}\n", features.join(", ")));
    text
}

pub fn routes() -> Result<Vec<Route>, Error> {
    Ok(vec![
        route::<NoArgs, HealthReport, _>(&groups::PG_HEALTH, HEALTH_DESCRIPTION, health)?,
        route::<NoArgs, DoctorReport, _>(&groups::PG_DOCTOR, DOCTOR_DESCRIPTION, doctor)?,
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

    #[test]
    fn origins_render_in_kebab_case() {
        assert_eq!(origin_name(Origin::Environment), "environment");
        assert_eq!(origin_name(Origin::Libpq), "libpq");
    }
}
