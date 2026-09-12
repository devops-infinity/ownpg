use clap::CommandFactory;
use ownpg_core::{Error, ExitClass, Result};

use crate::cli::{Cli, Command, ServeArgs, ShellArg};
use crate::output::{emit, report_error, stdout_error};
use crate::{config_cmd, context, doctor, logging, serve};

pub(crate) fn run(args: Cli) -> ExitClass {
    let outcome = dispatch(args);
    match outcome {
        Ok(class) => class,
        Err(error) => {
            report_error(&error);
            error.exit_class()
        }
    }
}

fn dispatch(args: Cli) -> Result<ExitClass> {
    match &args.command {
        Some(Command::Man { command }) => return show_manual(command),
        Some(Command::Completions { shell }) => return show_completions(*shell),
        _ => {}
    }
    let process = context::detect(&args.global)?;
    let human = !matches!(args.command, None | Some(Command::Serve(_)));
    let _log_guard = logging::init(&args.global, &process.paths, human)?;
    match args.command {
        None => {
            let defaults = ServeArgs {
                connection: crate::cli::ConnectionArgs::default(),
                no_audit: false,
                audit_path: None,
                pg_bindir: None,
                output_dir: None,
                http: false,
                bind: None,
                auth: None,
            };
            serve::run(&args.global, &defaults, &process)
        }
        Some(Command::Serve(serve_args)) => serve::run(&args.global, &serve_args, &process),
        Some(Command::Doctor(doctor_args)) => doctor::run(&args.global, &doctor_args, &process),
        Some(Command::Config(config)) => config_cmd::run(&args.global, &config, &process),
        Some(Command::Audit(crate::cli::AuditCommand::Verify { path })) => {
            config_cmd::verify_audit(&path)
        }
        Some(Command::Man { .. } | Command::Completions { .. }) => Ok(ExitClass::Success),
    }
}

fn show_manual(path: &[String]) -> Result<ExitClass> {
    let root = Cli::command();
    let mut current = root.clone();
    let mut walked: Vec<String> = Vec::new();
    for step in path {
        let Some(found) = current.get_subcommands().find(|sub| sub.get_name() == step) else {
            let known = current
                .get_subcommands()
                .map(|sub| sub.get_name().to_owned())
                .collect();
            return Err(Error::CommandUnknown {
                name: step.clone(),
                known,
            });
        };
        let next = found.clone();
        current = next;
        walked.push(step.clone());
    }

    let page = if walked.is_empty() {
        root
    } else {
        let titled = format!("ownpg-{}", walked.join("-"));
        let leaked: &'static str = Box::leak(titled.into_boxed_str());
        current.name(leaked).version(ownpg_core::VERSION)
    };

    let mut rendered = Vec::new();
    crate::man::render(&page, &mut rendered).map_err(stdout_error)?;
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
        assert!(error.remedy().contains("serve"));
    }

    #[test]
    fn a_nested_manual_page_renders() {
        let outcome = show_manual(&["config".to_owned(), "show".to_owned()]);
        assert!(outcome.is_ok());
    }
}
