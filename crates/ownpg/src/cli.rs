use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "ownpg",
    version,
    about = "PostgreSQL DBA tools for AI clients over the Model Context Protocol",
    long_about = "Serve one PostgreSQL database and one schema to an AI client over the\n\
                  Model Context Protocol, in read-only, write-only, or read-write mode.\n\n\
                  This build carries the command surface, the manual pages, and the shell\n\
                  completions. The server itself arrives in the next release.",
    after_help = "EXIT CODES:\n  \
        0 success   1 runtime failure   2 usage or configuration   4 refused by policy\n  \
        5 external failure   130 interrupted   101 a bug\n\n\
        Documentation: https://github.com/devops-infinity/ownpg-releases\n  \
        Report a bug: https://github.com/devops-infinity/ownpg-releases/issues/new",
    disable_help_subcommand = true,
    infer_subcommands = false,
    propagate_version = true,
    max_term_width = 100
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
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

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_subcommand_has_a_help_line() {
        for sub in Cli::command().get_subcommands() {
            assert!(
                sub.get_about().is_some(),
                "{} has no help line",
                sub.get_name()
            );
        }
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
}
