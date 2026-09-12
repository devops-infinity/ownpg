use rmcp::model::ToolAnnotations;

use crate::config::{Mode, Settings, ToolGroup};

pub const SCOPE_READ: &str = "ownpg:read";
pub const SCOPE_WRITE: &str = "ownpg:write";
pub const SCOPE_DDL: &str = "ownpg:ddl";
pub const SCOPE_ROLES: &str = "ownpg:roles";
pub const SCOPE_MAINTENANCE: &str = "ownpg:maintenance";
pub const SCOPE_HOST: &str = "ownpg:host";

const ALL_MODES: &[Mode] = &[Mode::ReadOnly, Mode::WriteOnly, Mode::ReadWrite];
const READ_MODES: &[Mode] = &[Mode::ReadOnly, Mode::ReadWrite];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub group: Option<ToolGroup>,
    pub modes: &'static [Mode],
    pub scope: &'static str,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

impl ToolSpec {
    #[must_use]
    pub fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::with_title(self.title)
            .read_only(self.read_only)
            .destructive(self.destructive)
            .idempotent(self.idempotent)
            .open_world(false)
    }

    #[must_use]
    pub fn allowed_in(&self, mode: Mode) -> bool {
        self.modes.contains(&mode)
    }

    #[must_use]
    pub fn loads_for(&self, settings: &Settings) -> bool {
        let mode = settings.mode.value;
        if !self.allowed_in(mode) {
            return false;
        }
        match self.group {
            None => true,
            Some(group) => settings.loaded_groups().contains(&group),
        }
    }
}

const fn read_tool(name: &'static str, title: &'static str, modes: &'static [Mode]) -> ToolSpec {
    ToolSpec {
        name,
        title,
        group: None,
        modes,
        scope: SCOPE_READ,
        read_only: true,
        destructive: false,
        idempotent: true,
    }
}

pub const PG_LIST_OBJECTS: ToolSpec = read_tool(
    "pg_list_objects",
    "List objects in the scoped schema",
    ALL_MODES,
);
pub const PG_DESCRIBE: ToolSpec =
    read_tool("pg_describe", "Describe one database object", ALL_MODES);
pub const PG_RUN_QUERY: ToolSpec = read_tool("pg_run_query", "Run one read statement", READ_MODES);
pub const PG_COUNT: ToolSpec = read_tool("pg_count", "Count rows in a table", READ_MODES);
pub const PG_EXPLAIN: ToolSpec = read_tool("pg_explain", "Explain a statement's plan", READ_MODES);
pub const PG_HEALTH: ToolSpec = read_tool("pg_health", "Summarize server health", ALL_MODES);
pub const PG_DOCTOR: ToolSpec = read_tool(
    "pg_doctor",
    "Report the connection and its settings",
    ALL_MODES,
);

pub const TOOLS: &[ToolSpec] = &[
    PG_LIST_OBJECTS,
    PG_DESCRIBE,
    PG_RUN_QUERY,
    PG_COUNT,
    PG_EXPLAIN,
    PG_HEALTH,
    PG_DOCTOR,
];

#[must_use]
pub fn spec(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|tool| tool.name == name)
}

#[must_use]
pub fn loaded(settings: &Settings) -> Vec<&'static ToolSpec> {
    TOOLS
        .iter()
        .filter(|tool| tool.loads_for(settings))
        .collect()
}

#[must_use]
pub const fn scope_for(group: Option<ToolGroup>) -> &'static str {
    match group {
        None | Some(ToolGroup::Monitoring) => SCOPE_READ,
        Some(ToolGroup::Write | ToolGroup::Transactions) => SCOPE_WRITE,
        Some(ToolGroup::Ddl) => SCOPE_DDL,
        Some(ToolGroup::Roles) => SCOPE_ROLES,
        Some(ToolGroup::Maintenance) => SCOPE_MAINTENANCE,
        Some(ToolGroup::Host) => SCOPE_HOST,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_tool_appears_once_with_the_prefix_and_a_short_name() {
        let mut seen = BTreeSet::new();
        for tool in TOOLS {
            assert!(seen.insert(tool.name), "{} is listed twice", tool.name);
            assert!(
                tool.name.starts_with("pg_"),
                "{} lacks the prefix",
                tool.name
            );
            assert!(tool.name.len() < 64, "{} is too long", tool.name);
            assert!(!tool.title.is_empty(), "{} has no title", tool.name);
            assert!(!tool.modes.is_empty(), "{} allows no mode", tool.name);
        }
    }

    #[test]
    fn every_tool_carries_the_scope_of_its_group() {
        for tool in TOOLS {
            assert_eq!(tool.scope, scope_for(tool.group), "{}", tool.name);
        }
        for group in ToolGroup::ALL {
            assert!(scope_for(Some(group)).starts_with("ownpg:"));
        }
    }

    #[test]
    fn default_tools_are_read_only_and_closed_world() {
        for tool in TOOLS.iter().filter(|tool| tool.group.is_none()) {
            let annotations = tool.annotations();
            assert_eq!(annotations.read_only_hint, Some(true), "{}", tool.name);
            assert_eq!(annotations.open_world_hint, Some(false), "{}", tool.name);
            assert_eq!(annotations.title.as_deref(), Some(tool.title));
        }
    }

    #[test]
    fn a_read_tool_is_absent_in_write_only_mode() {
        let run_query = spec("pg_run_query").unwrap();
        assert!(run_query.allowed_in(Mode::ReadOnly));
        assert!(!run_query.allowed_in(Mode::WriteOnly));
        let describe = spec("pg_describe").unwrap();
        assert!(describe.allowed_in(Mode::WriteOnly));
        assert!(spec("pg_missing").is_none());
    }
}
