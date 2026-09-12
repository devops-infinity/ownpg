use std::io::Write;

use clap_mangen::Man;
use clap_mangen::roff::{Roff, bold, roman};

pub(crate) const EXIT_STATUS: &[(u8, &str)] = &[
    (0, "success"),
    (
        1,
        "a runtime failure, such as a statement the database refused",
    ),
    (2, "a usage or configuration error"),
    (
        4,
        "refused by policy: the mode, the scope, or the role check said no",
    ),
    (
        5,
        "an external failure: the database, the bastion, or a host program",
    ),
    (130, "interrupted from a terminal before the work finished"),
    (
        101,
        "a bug in OwnPG; the panic message names where to report it",
    ),
];

pub(crate) const ENVIRONMENT: &[(&str, &str)] = &[
    (
        "OWNPG_CONFIG",
        "the profile file to read instead of the platform default",
    ),
    (
        "OWNPG_PROFILE",
        "the profile name to load from the profile file",
    ),
    ("OWNPG_MODE", "read-only, write-only, or read-write"),
    (
        "OWNPG_DATABASE",
        "the one database this server manages (PGDATABASE also applies)",
    ),
    ("OWNPG_SCHEMA", "the one schema this server manages"),
    (
        "OWNPG_HOST, OWNPG_PORT, OWNPG_USER",
        "where and as whom to connect (PGHOST, PGPORT, PGUSER, PGHOSTADDR also apply)",
    ),
    (
        "OWNPG_DSN",
        "a libpq connection string; PGSERVICE names a pg_service.conf entry",
    ),
    (
        "OWNPG_PASSWORD",
        "the database password (PGPASSWORD and PGPASSFILE also apply)",
    ),
    (
        "OWNPG_SSLMODE, OWNPG_SSLROOTCERT, OWNPG_SSLCERT, OWNPG_SSLKEY",
        "TLS settings (the PGSSL* forms also apply)",
    ),
    (
        "OWNPG_CONNECT_TIMEOUT, OWNPG_STATEMENT_TIMEOUT, OWNPG_LOCK_TIMEOUT, OWNPG_TRANSACTION_TIMEOUT",
        "timeouts in seconds",
    ),
    (
        "OWNPG_HANDLE_EXPIRY, OWNPG_CURSOR_EXPIRY",
        "idle seconds before a transaction handle or a cursor is released",
    ),
    ("OWNPG_ROW_CAP, OWNPG_BYTE_CAP", "result caps"),
    (
        "OWNPG_POOLED",
        "true when the target is a transaction pooler",
    ),
    ("OWNPG_STRICT_ROLE", "refuse elevated roles"),
    ("OWNPG_TOOLS", "the tool groups to load"),
    ("OWNPG_NO_INPUT", "never prompt (CI=true also counts)"),
    (
        "OWNPG_AUDIT, OWNPG_AUDIT_PATH, OWNPG_AUDIT_MAX_BYTES, OWNPG_AUDIT_KEEP_FILES",
        "the audit log",
    ),
    (
        "OWNPG_PG_BINDIR, OWNPG_OUTPUT_DIR",
        "where the PostgreSQL host programs live and where their output goes",
    ),
    (
        "OWNPG_SSH, OWNPG_SSH_TRANSPORT, OWNPG_SSH_TRUST_NEW_HOST",
        "the SSH bastion route",
    ),
    (
        "OWNPG_BIND, OWNPG_PUBLIC_URL, OWNPG_ALLOWED_HOSTS, OWNPG_ALLOWED_ORIGINS, OWNPG_TRUSTED_PROXIES",
        "the HTTP listener",
    ),
    (
        "OWNPG_AUTH, OWNPG_BEARER_TOKENS, OWNPG_BEARER_TOKENS_FILE, OWNPG_OAUTH_ISSUER, OWNPG_OAUTH_JWKS_URL, OWNPG_OAUTH_AUDIENCE, OWNPG_STATE_KEY_FILE",
        "HTTP authentication",
    ),
    (
        "OWNPG_BODY_CAP_BYTES, OWNPG_RATE_LIMIT_PER_MINUTE, OWNPG_MAX_CONNECTIONS, OWNPG_POOL_SIZE, OWNPG_SHUTDOWN_SECONDS, OWNPG_OLDER_CLIENT_SESSIONS",
        "HTTP limits",
    ),
    ("OTEL_EXPORTER_OTLP_ENDPOINT", "where metrics go when set"),
    (
        "RUST_LOG",
        "extra log directives that compose with -v and -q",
    ),
    ("NO_COLOR", "honored; OwnPG never colors its output"),
];

pub(crate) fn render(command: &clap::Command, output: &mut dyn Write) -> std::io::Result<()> {
    let page = Man::new(command.clone());
    page.render_title(output)?;
    page.render_name_section(output)?;
    page.render_synopsis_section(output)?;
    page.render_description_section(output)?;
    if command
        .get_arguments()
        .any(|argument| !argument.is_hide_set())
    {
        page.render_options_section(output)?;
    }
    if command.has_subcommands() {
        page.render_subcommands_section(output)?;
    }
    render_exit_status(output)?;
    render_environment(output)?;
    page.render_version_section(output)?;
    render_bug_reporting(output)
}

fn render_exit_status(output: &mut dyn Write) -> std::io::Result<()> {
    let mut roff = Roff::default();
    roff.control("SH", ["EXIT STATUS"]);
    for (code, meaning) in EXIT_STATUS {
        roff.control("TP", []);
        roff.text([bold(code.to_string())]);
        roff.text([roman(*meaning)]);
    }
    roff.to_writer(output)
}

fn render_environment(output: &mut dyn Write) -> std::io::Result<()> {
    let mut roff = Roff::default();
    roff.control("SH", ["ENVIRONMENT"]);
    roff.text([roman(
        "Flags win over environment variables, which win over the profile file. `ownpg config init --dry-run` prints every profile key next to its variable.",
    )]);
    for (names, meaning) in ENVIRONMENT {
        roff.control("TP", []);
        roff.text([bold(*names)]);
        roff.text([roman(*meaning)]);
    }
    roff.to_writer(output)
}

fn render_bug_reporting(output: &mut dyn Write) -> std::io::Result<()> {
    let mut roff = Roff::default();
    roff.control("SH", ["REPORTING BUGS"]);
    roff.text([roman(format!(
        "Open an issue at {} and attach the output of `ownpg --version` and `ownpg doctor --format json`; secrets are shown as `set` in that report.",
        crate::build_info::ISSUES_URL
    ))]);
    roff.to_writer(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_page_carries_the_exit_status_and_environment_sections() {
        let mut rendered = Vec::new();
        render(
            &<crate::cli::Cli as clap::CommandFactory>::command(),
            &mut rendered,
        )
        .unwrap();
        let text = String::from_utf8(rendered).unwrap();
        for heading in [
            "\"EXIT STATUS\"",
            "ENVIRONMENT",
            "\"REPORTING BUGS\"",
            "OPTIONS",
            "SUBCOMMANDS",
        ] {
            assert!(
                text.contains(&format!(".SH {heading}")),
                "{heading} is missing"
            );
        }
        assert!(text.contains("130"));
        assert!(text.contains("OWNPG_DATABASE"));
    }

    #[test]
    fn the_exit_status_table_matches_the_exit_classes() {
        let codes: Vec<u8> = EXIT_STATUS.iter().map(|(code, _)| *code).collect();
        for class in [
            ownpg_core::ExitClass::Success,
            ownpg_core::ExitClass::Runtime,
            ownpg_core::ExitClass::Usage,
            ownpg_core::ExitClass::Refused,
            ownpg_core::ExitClass::External,
            ownpg_core::ExitClass::Interrupted,
        ] {
            assert!(
                codes.contains(&class.code()),
                "{} is not in the manual",
                class.code()
            );
        }
    }

    #[test]
    fn every_variable_the_resolver_reads_is_in_the_manual() {
        let sources = [
            include_str!("../../ownpg-core/src/config/resolve.rs"),
            include_str!("../../ownpg-core/src/config/http.rs"),
            include_str!("cli.rs"),
            include_str!("context.rs"),
        ];
        let listed: String = ENVIRONMENT
            .iter()
            .map(|(names, _)| *names)
            .collect::<Vec<_>>()
            .join(" ");
        for source in sources {
            for (index, _) in source.match_indices("\"OWNPG_") {
                let rest = &source[index + 1..];
                let end = rest.find('"').unwrap_or(rest.len());
                let variable = &rest[..end];
                if variable == "OWNPG_BUILD_COMMIT" || variable == "OWNPG_BUILD_DATE" {
                    continue;
                }
                assert!(
                    listed.contains(variable),
                    "{variable} is not in the ENVIRONMENT section"
                );
            }
        }
    }
}
