use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo::rerun-if-env-changed=OWNPG_BUILD_COMMIT");
    for path in git_state_files() {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    println!("cargo::rustc-env=OWNPG_BUILD_COMMIT={}", commit_hash());
    println!("cargo::rustc-env=OWNPG_BUILD_DATE={}", build_date());
}

fn workspace_root() -> Option<PathBuf> {
    env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .and_then(|dir| dir.parent()?.parent().map(Path::to_path_buf))
        .filter(|root| root.join(".git").is_dir())
}

fn git_state_files() -> Vec<PathBuf> {
    let Some(git_dir) = workspace_root().map(|root| root.join(".git")) else {
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

fn git_output(root: &Path, arguments: &[&str]) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn packaged_commit() -> Option<String> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from)?;
    let text = fs::read_to_string(manifest_dir.join(".cargo_vcs_info.json")).ok()?;
    let marker = "\"sha1\"";
    let start = text.find(marker)? + marker.len();
    let rest = text.get(start..)?;
    let quote = rest.find('"')? + 1;
    let hash: String = rest
        .get(quote..)?
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .take(12)
        .collect();
    (hash.len() == 12).then_some(hash)
}

fn commit_hash() -> String {
    if let Some(given) = env::var("OWNPG_BUILD_COMMIT")
        .ok()
        .map(|value| value.trim().chars().take(12).collect::<String>())
        .filter(|value| !value.is_empty())
    {
        return given;
    }
    workspace_root()
        .and_then(|root| git_output(&root, &["rev-parse", "--short=12", "HEAD"]))
        .or_else(packaged_commit)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn build_date() -> String {
    env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .or_else(|| {
            workspace_root()
                .and_then(|root| git_output(&root, &["log", "-1", "--format=%ct"]))
                .and_then(|text| text.parse::<i64>().ok())
        })
        .map(|epoch| {
            let (year, month, day) = civil_from_days(epoch.div_euclid(86_400));
            format!("{year:04}-{month:02}-{day:02}")
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[expect(
    clippy::integer_division,
    reason = "the civil date formula works in whole days"
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
