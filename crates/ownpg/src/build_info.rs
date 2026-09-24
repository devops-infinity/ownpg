use std::sync::OnceLock;

pub(crate) const ISSUES_URL: &str = "https://github.com/devops-infinity/ownpg-releases/issues/new";

pub(crate) fn version_line() -> &'static str {
    static LINE: OnceLock<String> = OnceLock::new();
    LINE.get_or_init(|| {
        format!(
            "{} (commit {}, built {})",
            env!("CARGO_PKG_VERSION"),
            env!("OWNPG_BUILD_COMMIT"),
            env!("OWNPG_BUILD_DATE")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_starts_with_the_crate_version_and_carries_the_stamp() {
        let line = version_line();
        assert!(line.starts_with(env!("CARGO_PKG_VERSION")));
        assert!(line.contains("commit "));
        assert!(line.contains("built "));
    }

    #[test]
    fn the_build_date_is_a_calendar_date() {
        let date = env!("OWNPG_BUILD_DATE");
        let widths: Vec<usize> = date.split('-').map(str::len).collect();
        assert_eq!(widths, [4, 2, 2]);
        assert!(date.chars().all(|c| c.is_ascii_digit() || c == '-'));
    }
}
