#![allow(
    clippy::print_stdout,
    reason = "cargo directives are written to stdout"
)]
#![allow(
    dead_code,
    reason = "the command tree is compiled a second time here, where only its clap definition is used"
)]

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::CommandFactory;

#[path = "src/cli.rs"]
mod cli;

fn main() -> io::Result<()> {
    println!("cargo::rerun-if-changed=src/cli.rs");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=SOURCE_DATE_EPOCH");
    for path in git_state_files() {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    println!("cargo::rustc-env=OWNPG_BUILD_COMMIT={}", commit());
    println!("cargo::rustc-env=OWNPG_BUILD_DATE={}", build_date());

    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return Ok(());
    };

    let manuals = out_dir.join("artifacts").join("man");
    fs::create_dir_all(&manuals)?;

    let mut command = cli::Cli::command();
    command.build();

    write_manuals(&command, &manuals)?;
    Ok(())
}

fn git_state_files() -> Vec<PathBuf> {
    let Some(git_dir) = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .and_then(|dir| dir.parent()?.parent().map(Path::to_path_buf))
        .map(|root| root.join(".git"))
        .filter(|dir| dir.is_dir())
    else {
        return Vec::new();
    };
    let head = git_dir.join("HEAD");
    let mut files = vec![head.clone()];
    if let Ok(content) = fs::read_to_string(&head)
        && let Some(reference) = content.trim().strip_prefix("ref: ")
    {
        let target = git_dir.join(reference);
        if target.is_file() {
            files.push(target);
        }
    }
    files
}

fn commit() -> String {
    Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn build_date() -> String {
    env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .map(|epoch| {
            let (year, month, day) = civil_from_days(epoch.div_euclid(86_400));
            format!("{year:04}-{month:02}-{day:02}")
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[allow(
    clippy::integer_division,
    reason = "calendar arithmetic on whole days is exact by construction"
)]
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn write_manuals(command: &clap::Command, target: &Path) -> io::Result<()> {
    let root = clap_mangen::Man::new(command.clone());
    let mut buffer = Vec::new();
    root.render(&mut buffer)?;
    fs::write(target.join("ownpg.1"), &buffer)?;

    write_nested(command, "ownpg", target)
}

fn write_nested(command: &clap::Command, prefix: &str, target: &Path) -> io::Result<()> {
    for sub in command.get_subcommands() {
        let name = format!("{prefix}-{}", sub.get_name());
        let file = format!("{name}.1");
        let leaked: &'static str = Box::leak(name.clone().into_boxed_str());
        let page = clap_mangen::Man::new(sub.clone().name(leaked));
        let mut buffer = Vec::new();
        page.render(&mut buffer)?;
        fs::write(target.join(file), &buffer)?;
        write_nested(sub, &name, target)?;
    }
    Ok(())
}
