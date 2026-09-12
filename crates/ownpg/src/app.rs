use clap::CommandFactory;
use ownpg_core::{Error, ExitClass, Result};

use crate::cli::{Cli, Command, ShellArg};
use crate::output::{emit, report_error, stdout_error};

pub(crate) fn run(args: Cli) -> ExitClass {
    let outcome = match args.command {
        Command::Man { command } => show_manual(&command),
        Command::Completions { shell } => show_completions(shell),
    };
    match outcome {
        Ok(class) => class,
        Err(error) => {
            report_error(&error);
            error.exit_class()
        }
    }
}

fn show_manual(path: &[String]) -> Result<ExitClass> {
    let root = Cli::command();
    let mut here = root.clone();
    let mut walked: Vec<String> = Vec::new();
    for step in path {
        let Some(found) = here.get_subcommands().find(|sub| sub.get_name() == step) else {
            let known = here
                .get_subcommands()
                .map(|sub| sub.get_name().to_owned())
                .collect();
            return Err(Error::CommandUnknown {
                name: step.clone(),
                known,
            });
        };
        let next = found.clone();
        here = next;
        walked.push(step.clone());
    }

    let page = if walked.is_empty() {
        clap_mangen::Man::new(root)
    } else {
        let titled = format!("ownpg-{}", walked.join("-"));
        let leaked: &'static str = Box::leak(titled.into_boxed_str());
        clap_mangen::Man::new(here.name(leaked).version(ownpg_core::VERSION))
    };

    let mut rendered = Vec::new();
    page.render(&mut rendered).map_err(stdout_error)?;
    emit(|out| out.write_all(&rendered)).map_err(stdout_error)?;
    Ok(ExitClass::Success)
}

fn show_completions(shell: ShellArg) -> Result<ExitClass> {
    let target = match shell {
        ShellArg::Bash => clap_complete::Shell::Bash,
        ShellArg::Elvish => clap_complete::Shell::Elvish,
        ShellArg::Fish => clap_complete::Shell::Fish,
        ShellArg::PowerShell => clap_complete::Shell::PowerShell,
        ShellArg::Zsh => clap_complete::Shell::Zsh,
    };

    let mut script = Vec::new();
    clap_complete::generate(target, &mut Cli::command(), "ownpg", &mut script);
    emit(|out| out.write_all(&script)).map_err(stdout_error)?;
    Ok(ExitClass::Success)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manual_path_that_names_no_command_is_refused_with_the_known_names() {
        let error = show_manual(&["mna".to_owned()]).unwrap_err();
        assert_eq!(error.id().as_str(), "command.unknown");
        assert_eq!(error.exit_class(), ExitClass::Usage);
        assert!(error.remedy().contains("man"));
        assert!(error.remedy().contains("completions"));
    }
}
