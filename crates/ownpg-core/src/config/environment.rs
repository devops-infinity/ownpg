use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Environment {
    vars: BTreeMap<String, String>,
    home: Option<PathBuf>,
    os_user: Option<String>,
}

const SECRET_NAME_ENDINGS: &[&str] = &[
    "PASSWORD",
    "PASSPHRASE",
    "TOKEN",
    "TOKENS",
    "SECRET",
    "KEY",
    "DSN",
];

impl std::fmt::Debug for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut vars = f.debug_map();
        for (name, value) in &self.vars {
            let upper = name.to_ascii_uppercase();
            if SECRET_NAME_ENDINGS
                .iter()
                .any(|ending| upper.ends_with(ending))
            {
                vars.entry(name, &"<redacted>");
            } else {
                vars.entry(name, value);
            }
        }
        vars.finish()?;
        f.debug_struct(" Environment")
            .field("home", &self.home)
            .field("os_user", &self.os_user)
            .finish()
    }
}

impl Environment {
    #[must_use]
    pub fn new(
        vars: BTreeMap<String, String>,
        home: Option<PathBuf>,
        os_user: Option<String>,
    ) -> Self {
        Self {
            vars,
            home,
            os_user,
        }
    }

    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
    }

    #[must_use]
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    #[must_use]
    pub fn os_user(&self) -> Option<&str> {
        self.os_user.as_deref()
    }

    #[must_use]
    pub fn in_home(&self, relative: &str) -> Option<PathBuf> {
        self.home.as_ref().map(|home| home.join(relative))
    }

    #[must_use]
    pub fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_owned(), value.to_owned());
        self
    }

    #[must_use]
    pub fn with_home(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
    }

    #[must_use]
    pub fn with_os_user(mut self, user: &str) -> Self {
        self.os_user = Some(user.to_owned());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_variable_reads_as_unset() {
        let env = Environment::default()
            .with_var("PGHOST", "  ")
            .with_var("PGPORT", "5433");
        assert_eq!(env.var("PGHOST"), None);
        assert_eq!(env.var("PGPORT"), Some("5433"));
        assert_eq!(env.var("PGUSER"), None);
    }

    #[test]
    fn debug_output_redacts_secret_looking_variables() {
        let env = Environment::default()
            .with_var("PGPASSWORD", "hunter2")
            .with_var("OWNPG_BEARER_TOKENS", "abc")
            .with_var("OWNPG_DSN", "postgresql://app:inline-secret@db.example/app")
            .with_var("PGHOST", "db.example");
        let rendered = format!("{env:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("abc"), "{rendered}");
        assert!(!rendered.contains("inline-secret"), "{rendered}");
        assert!(rendered.contains("db.example"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn paths_under_home_resolve_only_when_home_is_known() {
        let bare = Environment::default();
        assert_eq!(bare.in_home(".pgpass"), None);
        let homed = bare.with_home("/home/u".into());
        assert_eq!(
            homed.in_home(".pgpass"),
            Some(PathBuf::from("/home/u/.pgpass"))
        );
    }
}
