use std::sync::Arc;

use ownpg_core::config::{Settings, Sources, resolve};
use ownpg_core::engine::Engine;
use ownpg_core::server::audit_probe;
use ownpg_core::tools::health::{DoctorReport, doctor_report, render_doctor};
use ownpg_core::{Error, ExitClass, Result};
use serde::Serialize;

use crate::cli::{DoctorArgs, GlobalArgs, OutputFormatArg};
use crate::context::{self, Process};
use crate::output::{emit, stdout_error};

pub(crate) const SUPPORTED_MAJORS: std::ops::RangeInclusive<i32> = 14..=18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckStatus {
    Ok,
    Warning,
    Failed,
}

impl CheckStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Check {
    pub name: &'static str,
    pub status: CheckStatus,
    pub required: bool,
    pub detail: String,
}

pub(crate) const FORMAT_VERSION: u32 = 1;

const PUBLIC_CREATE_SQL: &str = "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) a WHERE n.nspname = 'public' AND a.grantee = 0 AND a.privilege_type = 'CREATE')";

#[derive(Debug, Serialize)]
pub(crate) struct Verdict {
    pub format_version: u32,
    pub version: &'static str,
    pub status: CheckStatus,
    pub checks: Vec<Check>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<DoctorReport>,
}

impl Verdict {
    fn overall(checks: &[Check]) -> CheckStatus {
        if checks
            .iter()
            .any(|check| check.required && check.status == CheckStatus::Failed)
        {
            CheckStatus::Failed
        } else if checks.iter().any(|check| check.status != CheckStatus::Ok) {
            CheckStatus::Warning
        } else {
            CheckStatus::Ok
        }
    }

    pub(crate) fn exit_class(&self) -> ExitClass {
        if self.status == CheckStatus::Failed {
            ExitClass::Runtime
        } else {
            ExitClass::Success
        }
    }
}

fn check(
    name: &'static str,
    status: CheckStatus,
    required: bool,
    detail: impl Into<String>,
) -> Check {
    Check {
        name,
        status,
        required,
        detail: detail.into(),
    }
}

pub(crate) fn run(global: &GlobalArgs, args: &DoctorArgs, process: &Process) -> Result<ExitClass> {
    let verdict = examine(global, args, process);
    match args.format {
        OutputFormatArg::Json => {
            let rendered =
                serde_json::to_string_pretty(&verdict).map_err(|error| Error::ProtocolFailed {
                    detail: format!("the report could not be serialized: {error}"),
                })?;
            emit(|out| writeln!(out, "{rendered}")).map_err(stdout_error)?;
        }
        OutputFormatArg::Text => {
            emit(|out| {
                if let Some(report) = &verdict.report {
                    out.write_all(render_doctor(report).as_bytes())?;
                }
                for check in &verdict.checks {
                    writeln!(
                        out,
                        "check {} [{}]{}: {}",
                        check.name,
                        check.status.as_str(),
                        if check.required { "" } else { " (optional)" },
                        check.detail
                    )?;
                }
                writeln!(out, "verdict: {}", verdict.status.as_str())
            })
            .map_err(stdout_error)?;
        }
    }
    Ok(verdict.exit_class())
}

fn examine(global: &GlobalArgs, args: &DoctorArgs, process: &Process) -> Verdict {
    let mut checks = Vec::new();
    let config_file = &process.paths.config_file;
    checks.push(if config_file.exists() {
        match ownpg_core::config::profile::open_permissions(config_file) {
            Ok(None) => check(
                "profile_file",
                CheckStatus::Ok,
                false,
                format!("{} is readable by its owner only", config_file.display()),
            ),
            Ok(Some(mode)) => check(
                "profile_file",
                CheckStatus::Failed,
                false,
                format!(
                    "{} has mode {mode:o}; run chmod 600 on it",
                    config_file.display()
                ),
            ),
            Err(error) => check(
                "profile_file",
                CheckStatus::Failed,
                false,
                error.to_string(),
            ),
        }
    } else {
        check(
            "profile_file",
            CheckStatus::Ok,
            false,
            format!(
                "{} is absent; flags, environment, and libpq sources apply",
                config_file.display()
            ),
        )
    });

    let flags = match context::flag_layer(&args.connection, global, None) {
        Ok(flags) => flags,
        Err(error) => {
            checks.push(check(
                "settings",
                CheckStatus::Failed,
                true,
                error.to_string(),
            ));
            return finish(checks, None);
        }
    };
    let lookup = context::keychain_lookup;
    let (settings, warnings) = match resolve(
        flags,
        Sources {
            env: &process.env,
            paths: process.paths.clone(),
            keychain: Some(&lookup),
        },
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            checks.push(check(
                "settings",
                CheckStatus::Failed,
                true,
                format!("{error}. {}", error.remedy()),
            ));
            return finish(checks, None);
        }
    };
    let settings = Arc::new(settings);
    checks.push(check(
        "settings",
        CheckStatus::Ok,
        true,
        format!(
            "database {}, schema {}, mode {}",
            settings.database.value, settings.schema.value, settings.mode.value
        ),
    ));
    for warning in &warnings {
        checks.push(check(
            "settings",
            CheckStatus::Warning,
            false,
            warning.message.clone(),
        ));
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            checks.push(check(
                "runtime",
                CheckStatus::Failed,
                true,
                format!("the runtime could not start: {error}"),
            ));
            return finish(checks, None);
        }
    };
    let ssh_hints = context::ssh_hints(&process.env);
    let report = runtime.block_on(async {
        let engine = match Engine::start(Arc::clone(&settings), ssh_hints).await {
            Ok(engine) => engine,
            Err(error) => {
                let attempts = match &error {
                    Error::ConnectFailed { tried, .. } => tried.join("; "),
                    _ => String::new(),
                };
                let causes = crate::output::cause_chain(&error);
                let mut detail = error.to_string();
                if !attempts.is_empty() {
                    detail.push_str(&format!(" (tried {attempts})"));
                }
                if !causes.is_empty() {
                    detail.push_str(&format!("; caused by: {}", causes.join(" <- ")));
                }
                detail.push_str(&format!(". {}", error.remedy()));
                checks.push(check("connection", CheckStatus::Failed, true, detail));
                return None;
            }
        };
        run_connected_checks(&engine, &settings, &mut checks).await
    });
    finish(checks, report)
}

async fn run_connected_checks(
    engine: &Engine,
    settings: &Settings,
    checks: &mut Vec<Check>,
) -> Option<DoctorReport> {
    let info = engine.info().await;
    checks.push(check(
        "connection",
        CheckStatus::Ok,
        true,
        format!("{} via {} as {}", info.target, info.via.as_str(), info.role),
    ));
    checks.push(match info.tls_warning() {
        Some(warning) => check("tls", CheckStatus::Warning, false, warning),
        None => check(
            "tls",
            CheckStatus::Ok,
            false,
            match info.tls_used {
                Some(true) => "encrypted and verified as configured".to_owned(),
                Some(false) => "not encrypted, as configured".to_owned(),
                None => "not applicable on a Unix socket".to_owned(),
            },
        ),
    });
    let major = info.server_version_num.div_euclid(10_000);
    checks.push(if SUPPORTED_MAJORS.contains(&major) {
        check(
            "server_version",
            CheckStatus::Ok,
            false,
            format!("PostgreSQL {}", info.server_version),
        )
    } else {
        check(
            "server_version",
            CheckStatus::Warning,
            false,
            format!(
                "PostgreSQL {} is outside the tested range {} to {}",
                info.server_version,
                SUPPORTED_MAJORS.start(),
                SUPPORTED_MAJORS.end()
            ),
        )
    });
    match engine.role().await {
        Ok(role) => checks.push(match role.warning() {
            Some(warning) => check("role", CheckStatus::Warning, false, warning),
            None => check(
                "role",
                CheckStatus::Ok,
                false,
                format!("{} carries no elevated attribute", role.name),
            ),
        }),
        Err(error) => checks.push(check("role", CheckStatus::Failed, true, error.to_string())),
    }
    checks.push(check(
        "search_path",
        CheckStatus::Ok,
        false,
        if info.pooled {
            "empty; the connection is pooled and every name must be qualified".to_owned()
        } else {
            info.search_path.clone()
        },
    ));
    checks.push(match engine.catalog_rows(PUBLIC_CREATE_SQL, &[]).await {
        Ok(rows) => {
            let open = rows
                .first()
                .and_then(|row| row.try_get::<_, bool>(0).ok())
                .unwrap_or(false);
            if open {
                check(
                    "public_schema",
                    CheckStatus::Warning,
                    false,
                    "schema public grants CREATE to PUBLIC; run REVOKE CREATE ON SCHEMA public FROM PUBLIC".to_owned(),
                )
            } else {
                check(
                    "public_schema",
                    CheckStatus::Ok,
                    false,
                    "schema public grants no CREATE to PUBLIC".to_owned(),
                )
            }
        }
        Err(error) => check("public_schema", CheckStatus::Warning, false, error.to_string()),
    });
    let audit_path = match audit_probe(settings) {
        Ok(path) => {
            checks.push(match &path {
                Some(path) => check(
                    "audit",
                    CheckStatus::Ok,
                    false,
                    format!("writable at {}", path.display()),
                ),
                None => check("audit", CheckStatus::Warning, false, "off"),
            });
            path
        }
        Err(error) => {
            checks.push(check(
                "audit",
                CheckStatus::Failed,
                false,
                error.to_string(),
            ));
            None
        }
    };
    match doctor_report(engine, audit_path.as_deref()).await {
        Ok(report) => {
            checks.push(check(
                "tools",
                CheckStatus::Ok,
                false,
                report.tools.join(", "),
            ));
            for program in &report.host_programs {
                let name: &'static str = ownpg_core::tools::host::PROGRAMS
                    .iter()
                    .copied()
                    .find(|known| *known == program.name)
                    .unwrap_or("host_program");
                checks.push(match (&program.path, &program.version) {
                    (Some(path), Some(version)) => {
                        check(name, CheckStatus::Ok, false, format!("{path} ({version})"))
                    }
                    (Some(path), None) => check(
                        name,
                        CheckStatus::Warning,
                        false,
                        format!("{path} (the version could not be read)"),
                    ),
                    (None, _) => check(
                        name,
                        CheckStatus::Warning,
                        false,
                        "not found through pg_bindir or an absolute PATH entry".to_owned(),
                    ),
                });
            }
            Some(report)
        }
        Err(error) => {
            checks.push(check(
                "report",
                CheckStatus::Failed,
                false,
                error.to_string(),
            ));
            None
        }
    }
}

fn finish(checks: Vec<Check>, report: Option<DoctorReport>) -> Verdict {
    Verdict {
        format_version: FORMAT_VERSION,
        version: crate::build_info::version_line(),
        status: Verdict::overall(&checks),
        checks,
        report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_required_failure_fails_the_verdict_and_a_warning_does_not() {
        let warned = finish(
            vec![
                check("a", CheckStatus::Ok, true, ""),
                check("b", CheckStatus::Warning, false, ""),
            ],
            None,
        );
        assert_eq!(warned.status, CheckStatus::Warning);
        assert_eq!(warned.exit_class(), ExitClass::Success);
        let failed = finish(vec![check("a", CheckStatus::Failed, true, "")], None);
        assert_eq!(failed.status, CheckStatus::Failed);
        assert_eq!(failed.exit_class(), ExitClass::Runtime);
        let optional = finish(vec![check("a", CheckStatus::Failed, false, "")], None);
        assert_eq!(optional.status, CheckStatus::Warning);
    }
}
