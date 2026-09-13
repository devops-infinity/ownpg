use std::path::PathBuf;

use clap::{
    ArgAction, Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum, ValueHint,
};

#[derive(Debug, Parser)]
#[command(
    name = "ownpg",
    version,
    about = "PostgreSQL DBA tools for AI clients over the Model Context Protocol",
    long_about = "OwnPG serves one PostgreSQL database and one schema to an AI client over\n\
                  the Model Context Protocol, in read-only, write-only, or read-write mode.\n\n\
                  With no command, `ownpg` serves over stdio with the default settings.",
    after_help = "EXIT CODES:\n  \
        0 success   1 runtime failure   2 usage or configuration   4 refused by policy\n  \
        5 external failure   130 interrupted   101 a bug\n\n\
        Documentation: https://github.com/devops-infinity/ownpg-releases\n  \
        Report a bug: https://github.com/devops-infinity/ownpg-releases/issues/new",
    after_long_help = ROOT_EXAMPLES,
    disable_help_subcommand = true,
    infer_subcommands = false,
    propagate_version = true,
    max_term_width = 100
)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Args)]
pub(crate) struct GlobalArgs {
    #[arg(
        short = 'v',
        long,
        global = true,
        action = ArgAction::Count,
        help = "More log detail on stderr; repeat for trace. Composes with RUST_LOG"
    )]
    pub verbose: u8,

    #[arg(
        short = 'q',
        long,
        global = true,
        conflicts_with = "verbose",
        help = "Only errors on stderr"
    )]
    pub quiet: bool,

    #[arg(
        long,
        global = true,
        help = "Never prompt; fail instead. Implied by CI=true [env: OWNPG_NO_INPUT]"
    )]
    pub no_input: bool,

    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "FORMAT",
        default_value_t = LogFormatArg::Text,
        help = "Shape of the log lines on stderr"
    )]
    pub log_format: LogFormatArg,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        value_hint = ValueHint::FilePath,
        help = "Also write logs to this file, rotated daily with the newest eight files kept. A bare name lands under the log directory (see `config path`)"
    )]
    pub log_file: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        env = "OWNPG_CONFIG",
        value_name = "FILE",
        value_hint = ValueHint::FilePath,
        help = "Profile file to read instead of the one in the config directory"
    )]
    pub config: Option<PathBuf>,
}

pub(crate) const ROOT_EXAMPLES: &str = "EXAMPLES:\n  \
    ownpg serve -d app                  serve app.public read-only over stdio\n  \
    ownpg serve -d app -s billing -m rw --tools write,transactions\n  \
    ownpg doctor -p staging --format json\n  \
    ownpg serve --http --auth bearer --bind 127.0.0.1:8765 -d app\n  \
    ownpg config init && ownpg config set-password local\n  \
    ownpg completions zsh > ~/.zfunc/_ownpg\n\n\
    EXIT CODES:\n  \
    0 success   1 runtime failure   2 usage or configuration   4 refused by policy\n  \
    5 external failure   130 interrupted   101 a bug\n\n\
    Documentation: https://github.com/devops-infinity/ownpg-releases\n  \
    Report a bug: https://github.com/devops-infinity/ownpg-releases/issues/new";

pub(crate) const SERVE_EXAMPLES: &str = "EXAMPLES:\n  \
    ownpg serve -d app                  stdio, read-only, the public schema\n  \
    ownpg serve -d app -m ro --ssh deploy@bastion.example\n  \
    ownpg serve --http --auth none --bind 127.0.0.1:8765 -d app\n  \
    ownpg serve --http --auth bearer --bind 0.0.0.0:8765 -d app --strict-role";

pub(crate) const DOCTOR_EXAMPLES: &str = "EXAMPLES:\n  \
    ownpg doctor -d app\n  \
    ownpg doctor -p staging --format json";

pub(crate) const CONFIG_EXAMPLES: &str = "EXAMPLES:\n  \
    ownpg config path\n  \
    ownpg config init --dry-run         print the starter profile file\n  \
    ownpg config show -p staging --format json\n  \
    ownpg config set-password staging   read the password from the terminal or stdin";

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    #[command(
        about = "Serve the database to an MCP client over stdio (the default command)",
        after_long_help = SERVE_EXAMPLES
    )]
    Serve(ServeArgs),

    #[command(
        about = "Check the connection and the settings, and report every attempt",
        after_long_help = DOCTOR_EXAMPLES
    )]
    Doctor(DoctorArgs),

    #[command(
        subcommand,
        about = "Manage connection profiles and local state",
        after_long_help = CONFIG_EXAMPLES
    )]
    Config(ConfigCommand),

    #[command(subcommand, about = "Work with the audit log")]
    Audit(AuditCommand),

    #[command(about = "Write the manual page to stdout")]
    Man {
        #[arg(
            value_name = "COMMAND",
            help = "Write the page for one command instead of the whole tool. A nested one is named in full, as in `config show`"
        )]
        command: Vec<String>,
    },

    #[command(about = "Write a shell completion script to stdout")]
    Completions {
        #[arg(value_enum, help = "The shell to generate for")]
        shell: ShellArg,
    },
}

#[derive(Debug, Clone, Default, Args)]
pub(crate) struct ConnectionArgs {
    #[arg(
        short = 'p',
        long,
        value_name = "NAME",
        help = "Profile to read from the profile file [env: OWNPG_PROFILE]"
    )]
    pub profile: Option<String>,

    #[arg(
        short = 'm',
        long,
        value_enum,
        value_name = "MODE",
        help = "Access mode for this run [env: OWNPG_MODE]"
    )]
    pub mode: Option<ModeArg>,

    #[arg(
        short = 'd',
        long,
        value_name = "NAME",
        help = "Database to serve [env: OWNPG_DATABASE]"
    )]
    pub database: Option<String>,

    #[arg(
        short = 's',
        long,
        value_name = "NAME",
        help = "Schema every call is scoped to (default public) [env: OWNPG_SCHEMA]"
    )]
    pub schema: Option<String>,

    #[arg(
        long,
        value_name = "HOST",
        value_hint = ValueHint::Hostname,
        help = "Host name, address, or Unix socket directory [env: OWNPG_HOST]"
    )]
    pub host: Option<String>,

    #[arg(
        long,
        value_name = "PORT",
        value_parser = clap::value_parser!(u16).range(1..),
        help = "TCP port, or the socket file suffix [env: OWNPG_PORT]"
    )]
    pub port: Option<u16>,

    #[arg(
        short = 'U',
        long,
        value_name = "ROLE",
        help = "Role to connect as [env: OWNPG_USER]"
    )]
    pub user: Option<String>,

    #[arg(
        long,
        value_enum,
        value_name = "MODE",
        help = "TLS requirement, with the libpq meaning of each value [env: OWNPG_SSLMODE]"
    )]
    pub sslmode: Option<SslModeArg>,

    #[arg(
        long,
        value_name = "FILE",
        value_hint = ValueHint::FilePath,
        help = "Root certificate file, or `system` for the platform trust store [env: OWNPG_SSLROOTCERT]"
    )]
    pub sslrootcert: Option<PathBuf>,

    #[arg(
        long,
        value_name = "GROUPS",
        value_delimiter = ',',
        help = "Extra tool groups to load: write, transactions, ddl, roles, maintenance, monitoring, host [env: OWNPG_TOOLS]"
    )]
    pub tools: Vec<String>,

    #[arg(
        long,
        help = "Refuse to start on a superuser, rds_superuser, or BYPASSRLS role; on by default with --http, off otherwise [env: OWNPG_STRICT_ROLE]"
    )]
    pub strict_role: bool,

    #[arg(
        long,
        value_name = "[USER@]HOST[:PORT]",
        help = "Reach PostgreSQL through this SSH bastion [env: OWNPG_SSH]"
    )]
    pub ssh: Option<String>,

    #[arg(
        long,
        value_enum,
        value_name = "TRANSPORT",
        help = "SSH client: the built-in one, or the system ssh command [env: OWNPG_SSH_TRANSPORT]"
    )]
    pub ssh_transport: Option<SshTransportArg>,

    #[arg(
        long,
        help = "Record an unknown bastion host key on first use [env: OWNPG_SSH_TRUST_NEW_HOST]"
    )]
    pub ssh_trust_new_host: bool,
}

#[derive(Debug, Clone, Args)]
pub(crate) struct ServeArgs {
    #[command(flatten)]
    pub connection: ConnectionArgs,

    #[arg(
        long,
        help = "Turn the audit log off for this run [env: OWNPG_AUDIT=false]"
    )]
    pub no_audit: bool,

    #[arg(
        long,
        value_name = "FILE",
        value_hint = ValueHint::FilePath,
        help = "Write the audit log here instead of the data directory [env: OWNPG_AUDIT_PATH]"
    )]
    pub audit_path: Option<PathBuf>,

    #[arg(
        long,
        value_name = "DIR",
        value_hint = ValueHint::DirPath,
        help = "Directory holding pg_dump and the other PostgreSQL programs [env: OWNPG_PG_BINDIR]"
    )]
    pub pg_bindir: Option<PathBuf>,

    #[arg(
        long,
        value_name = "DIR",
        value_hint = ValueHint::DirPath,
        help = "Directory the host-binary tools write files into [env: OWNPG_OUTPUT_DIR]"
    )]
    pub output_dir: Option<PathBuf>,

    #[arg(long, help = "Serve Streamable HTTP instead of stdio")]
    pub http: bool,

    #[arg(
        long,
        requires = "http",
        value_name = "ADDR",
        help = "Address to bind in HTTP mode: host:port, a bare address, or :port (default 127.0.0.1:8765) [env: OWNPG_BIND]"
    )]
    pub bind: Option<String>,

    #[arg(
        long,
        requires = "http",
        value_enum,
        value_name = "MODE",
        help = "How HTTP clients prove who they are [env: OWNPG_AUTH]"
    )]
    pub auth: Option<AuthArg>,
}

#[derive(Debug, Clone, Args)]
pub(crate) struct DoctorArgs {
    #[command(flatten)]
    pub connection: ConnectionArgs,

    #[arg(
        long,
        value_enum,
        value_name = "FORMAT",
        default_value_t = OutputFormatArg::Text,
        help = "Report shape"
    )]
    pub format: OutputFormatArg,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    #[command(about = "Print every setting with the layer it came from; secrets show as `set`")]
    Show {
        #[command(flatten)]
        connection: ConnectionArgs,

        #[arg(
            long,
            value_enum,
            value_name = "FORMAT",
            default_value_t = OutputFormatArg::Text,
            help = "Report shape"
        )]
        format: OutputFormatArg,
    },

    #[command(about = "Print the profile file path and the data, cache, and log directories")]
    Path {
        #[arg(
            long,
            value_enum,
            value_name = "FORMAT",
            default_value_t = OutputFormatArg::Text,
            help = "Report shape"
        )]
        format: OutputFormatArg,
    },

    #[command(about = "Write an example profile file")]
    Init {
        #[arg(long, help = "Replace a profile file that already exists")]
        force: bool,

        #[arg(long, help = "Print the file to stdout instead of writing it")]
        dry_run: bool,
    },

    #[command(
        name = "set-password",
        about = "Store a profile's password in the platform keychain, read without echo"
    )]
    SetPassword {
        #[arg(value_name = "PROFILE", help = "Profile the password belongs to")]
        profile: String,
        #[arg(
            long,
            help = "Say what would change without touching the keychain or the file"
        )]
        dry_run: bool,
    },

    #[command(
        name = "unset-password",
        about = "Remove a profile's password from the platform keychain"
    )]
    UnsetPassword {
        #[arg(value_name = "PROFILE", help = "Profile whose password is removed")]
        profile: String,
        #[arg(
            long,
            help = "Say what would change without touching the keychain or the file"
        )]
        dry_run: bool,
    },

    #[command(
        name = "set-ssh-passphrase",
        about = "Store a profile's SSH key passphrase in the platform keychain, read without echo"
    )]
    SetSshPassphrase {
        #[arg(
            value_name = "PROFILE",
            help = "Profile whose ssh section uses the key"
        )]
        profile: String,
        #[arg(
            long,
            help = "Say what would change without touching the keychain or the file"
        )]
        dry_run: bool,
    },

    #[command(
        name = "unset-ssh-passphrase",
        about = "Remove a profile's SSH key passphrase from the platform keychain"
    )]
    UnsetSshPassphrase {
        #[arg(value_name = "PROFILE", help = "Profile whose passphrase is removed")]
        profile: String,
        #[arg(
            long,
            help = "Say what would change without touching the keychain or the file"
        )]
        dry_run: bool,
    },

    #[command(name = "cache-clear", about = "Delete the cache directory contents")]
    CacheClear {
        #[arg(long, help = "List what would be removed without removing it")]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuditCommand {
    #[command(about = "Check that every line of an audit log chains to the one before it")]
    Verify {
        #[arg(value_name = "FILE", value_hint = ValueHint::FilePath, help = "The audit log to check")]
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ModeArg {
    #[value(name = "read-only", alias = "ro")]
    ReadOnly,
    #[value(name = "write-only", alias = "wo")]
    WriteOnly,
    #[value(name = "read-write", alias = "rw")]
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SslModeArg {
    Disable,
    Allow,
    Prefer,
    Require,
    #[value(name = "verify-ca")]
    VerifyCa,
    #[value(name = "verify-full")]
    VerifyFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SshTransportArg {
    #[value(name = "in-process")]
    InProcess,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum AuthArg {
    None,
    Bearer,
    Oauth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum LogFormatArg {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormatArg {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ShellArg {
    Bash,
    Elvish,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
    Zsh,
}

impl Cli {
    #[must_use]
    pub(crate) fn parse_args(version_line: &'static str) -> Self {
        let matches = Self::command().version(version_line).get_matches();
        match Self::from_arg_matches(&matches) {
            Ok(parsed) => parsed,
            Err(error) => error.exit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("ownpg").chain(args.iter().copied()))
    }

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_example_line_parses_through_the_command_tree() {
        let mut checked = 0;
        for block in [
            ROOT_EXAMPLES,
            SERVE_EXAMPLES,
            DOCTOR_EXAMPLES,
            CONFIG_EXAMPLES,
        ] {
            for line in block.lines() {
                let trimmed = line.trim();
                let Some(rest) = trimmed.strip_prefix("ownpg ") else {
                    continue;
                };
                let command = rest
                    .split("  ")
                    .next()
                    .unwrap_or_default()
                    .split(" && ")
                    .next()
                    .unwrap_or_default()
                    .split(" > ")
                    .next()
                    .unwrap_or_default();
                let words: Vec<&str> = command.split_whitespace().collect();
                assert!(
                    parse(&words).is_ok(),
                    "example does not parse: ownpg {command}"
                );
                checked += 1;
            }
        }
        assert!(checked >= 12, "{checked}");
    }

    #[test]
    fn every_subcommand_has_a_help_line() {
        fn check(command: &clap::Command) {
            for sub in command.get_subcommands() {
                assert!(
                    sub.get_about().is_some(),
                    "{} has no help line",
                    sub.get_name()
                );
                check(sub);
            }
        }
        check(&Cli::command());
    }

    #[test]
    fn the_help_footer_names_every_exit_code() {
        let help = Cli::command().render_long_help().to_string();
        for code in [
            "0 success",
            "1 runtime failure",
            "2 usage",
            "4 refused",
            "5 external",
            "130 interrupted",
        ] {
            assert!(help.contains(code), "help footer lacks {code:?}");
        }
        assert!(help.contains("https://github.com/devops-infinity/ownpg-releases/issues/new"));
    }

    #[test]
    fn no_command_means_serve_and_serve_accepts_the_connection_flags() {
        let bare = parse(&[]).unwrap();
        assert!(bare.command.is_none());
        let served = parse(&[
            "serve",
            "--mode",
            "rw",
            "-d",
            "app",
            "-s",
            "sales",
            "--tools",
            "ddl,roles",
            "--no-audit",
        ])
        .unwrap();
        let Some(Command::Serve(args)) = served.command else {
            panic!("serve was expected");
        };
        assert_eq!(args.connection.mode, Some(ModeArg::ReadWrite));
        assert_eq!(args.connection.database.as_deref(), Some("app"));
        assert_eq!(args.connection.schema.as_deref(), Some("sales"));
        assert_eq!(args.connection.tools, ["ddl", "roles"]);
        assert!(args.no_audit);
    }

    #[test]
    fn http_only_flags_need_http_and_quiet_conflicts_with_verbose() {
        assert!(parse(&["serve", "--bind", "0.0.0.0:1"]).is_err());
        assert!(parse(&["serve", "--auth", "none"]).is_err());
        assert!(parse(&["serve", "--http", "--auth", "bearer"]).is_ok());
        assert!(parse(&["-q", "-v", "doctor"]).is_err());
    }

    #[test]
    fn the_config_tree_names_every_documented_command() {
        for command in [
            "show",
            "path",
            "init",
            "set-password",
            "unset-password",
            "cache-clear",
        ] {
            let found = Cli::command()
                .find_subcommand("config")
                .and_then(|config| config.find_subcommand(command))
                .is_some();
            assert!(found, "config {command} is missing");
        }
        let parsed = parse(&["config", "set-password", "prod"]).unwrap();
        assert!(matches!(
            parsed.command,
            Some(Command::Config(ConfigCommand::SetPassword { profile, dry_run: false })) if profile == "prod"
        ));
        let cache_clear_dry_run = parse(&["config", "cache-clear", "--dry-run"]).unwrap();
        assert!(matches!(
            cache_clear_dry_run.command,
            Some(Command::Config(ConfigCommand::CacheClear { dry_run: true }))
        ));
        let verify = parse(&["audit", "verify", "/tmp/audit.jsonl"]).unwrap();
        assert!(matches!(
            verify.command,
            Some(Command::Audit(AuditCommand::Verify { .. }))
        ));
    }

    #[test]
    fn the_rendered_help_of_every_command_is_pinned() {
        fn collect_pages(
            command: &mut clap::Command,
            prefix: &str,
            pages: &mut Vec<(String, String)>,
        ) {
            let name = if prefix.is_empty() {
                command.get_name().to_owned()
            } else {
                format!("{prefix} {}", command.get_name())
            };
            let help = command.render_long_help().to_string();
            pages.push((name.clone(), help));
            for sub in command.get_subcommands_mut() {
                collect_pages(sub, &name, pages);
            }
        }
        let mut root = Cli::command();
        root.build();
        let mut pages = Vec::new();
        collect_pages(&mut root, "", &mut pages);
        assert!(pages.len() > 10, "{}", pages.len());
        let rendered: String = pages
            .iter()
            .map(|(name, help)| format!("===== {name} =====\n{help}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut normalized = String::with_capacity(rendered.len());
        let mut rest = rendered.as_str();
        while let Some(start) = rest.find("[env: OWNPG_CONFIG=") {
            normalized.push_str(&rest[..start]);
            normalized.push_str("[env: OWNPG_CONFIG]");
            let after = &rest[start..];
            rest = after.find(']').map_or("", |end| &after[end + 1..]);
        }
        normalized.push_str(rest);
        insta::assert_snapshot!("command-surface-help", normalized);
    }

    #[test]
    fn the_global_flags_parse_before_and_after_the_command() {
        let before = parse(&["-vv", "--log-format", "json", "doctor"]).unwrap();
        assert_eq!(before.global.verbose, 2);
        assert_eq!(before.global.log_format, LogFormatArg::Json);
        let after = parse(&["doctor", "--format", "json", "--no-input"]).unwrap();
        assert!(after.global.no_input);
        let Some(Command::Doctor(args)) = after.command else {
            panic!("doctor was expected");
        };
        assert_eq!(args.format, OutputFormatArg::Json);
    }
}
