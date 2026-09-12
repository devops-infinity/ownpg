use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    vars: BTreeMap<String, String>,
    home: Option<PathBuf>,
    os_user: Option<String>,
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
