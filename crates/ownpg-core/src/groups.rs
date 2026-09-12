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

const WRITE_MODES: &[Mode] = &[Mode::WriteOnly, Mode::ReadWrite];

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

const fn write_tool(
    name: &'static str,
    title: &'static str,
    group: ToolGroup,
    destructive: bool,
    idempotent: bool,
) -> ToolSpec {
    ToolSpec {
        name,
        title,
        group: Some(group),
        modes: WRITE_MODES,
        scope: scope_for(Some(group)),
        read_only: false,
        destructive,
        idempotent,
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

pub const PG_INSERT: ToolSpec =
    write_tool("pg_insert", "Insert rows", ToolGroup::Write, false, false);
pub const PG_UPDATE: ToolSpec =
    write_tool("pg_update", "Update rows", ToolGroup::Write, true, false);
pub const PG_DELETE: ToolSpec =
    write_tool("pg_delete", "Delete rows", ToolGroup::Write, true, true);
pub const PG_MERGE: ToolSpec = write_tool(
    "pg_merge",
    "Upsert rows with MERGE",
    ToolGroup::Write,
    true,
    false,
);
pub const PG_RUN_WRITE: ToolSpec = write_tool(
    "pg_run_write",
    "Run one write statement",
    ToolGroup::Write,
    true,
    false,
);
pub const PG_COPY: ToolSpec = write_tool(
    "pg_copy",
    "Load or return rows in bulk",
    ToolGroup::Write,
    false,
    false,
);
pub const PG_TRANSACTION: ToolSpec = write_tool(
    "pg_transaction",
    "Open, commit, or roll back a transaction",
    ToolGroup::Transactions,
    true,
    false,
);

const fn ddl_tool(name: &'static str, title: &'static str) -> ToolSpec {
    write_tool(name, title, ToolGroup::Ddl, true, false)
}

pub const PG_TABLE: ToolSpec = ddl_tool("pg_table", "Create, alter, or drop a table");
pub const PG_COLUMN: ToolSpec = ddl_tool("pg_column", "Change a column");
pub const PG_CONSTRAINT: ToolSpec = ddl_tool("pg_constraint", "Manage a constraint");
pub const PG_INDEX: ToolSpec = ddl_tool("pg_index", "Create, drop, or rebuild an index");
pub const PG_VIEW: ToolSpec = ddl_tool("pg_view", "Create, drop, or refresh a view");
pub const PG_SEQUENCE: ToolSpec = ddl_tool("pg_sequence", "Create, alter, or drop a sequence");
pub const PG_ROUTINE: ToolSpec = ddl_tool(
    "pg_routine",
    "Create, alter, or drop a function or procedure",
);
pub const PG_TRIGGER: ToolSpec = ddl_tool("pg_trigger", "Create, drop, or toggle a trigger");
pub const PG_TYPE: ToolSpec = ddl_tool("pg_type", "Create, alter, or drop a type");
pub const PG_EXTENSION: ToolSpec =
    ddl_tool("pg_extension", "Install, update, or drop an extension");
pub const PG_COMMENT: ToolSpec = write_tool(
    "pg_comment",
    "Set or remove a comment",
    ToolGroup::Ddl,
    false,
    true,
);
pub const PG_ROLE: ToolSpec = write_tool(
    "pg_role",
    "Create, alter, or drop a role",
    ToolGroup::Roles,
    true,
    false,
);
pub const PG_GRANT: ToolSpec = write_tool(
    "pg_grant",
    "Grant or revoke privileges",
    ToolGroup::Roles,
    true,
    true,
);
pub const PG_POLICY: ToolSpec = write_tool(
    "pg_policy",
    "Manage row-level security",
    ToolGroup::Roles,
    true,
    false,
);
pub const PG_PRIVILEGES: ToolSpec = write_tool(
    "pg_privileges",
    "List privileges or apply a template",
    ToolGroup::Roles,
    false,
    true,
);

const fn maintenance_tool(name: &'static str, title: &'static str, destructive: bool) -> ToolSpec {
    write_tool(name, title, ToolGroup::Maintenance, destructive, true)
}

const fn monitoring_tool(name: &'static str, title: &'static str) -> ToolSpec {
    ToolSpec {
        name,
        title,
        group: Some(ToolGroup::Monitoring),
        modes: READ_MODES,
        scope: SCOPE_READ,
        read_only: true,
        destructive: false,
        idempotent: true,
    }
}

pub const PG_VACUUM: ToolSpec = maintenance_tool("pg_vacuum", "Run VACUUM or CHECKPOINT", true);
pub const PG_ANALYZE: ToolSpec = maintenance_tool("pg_analyze", "Run ANALYZE", false);
pub const PG_REINDEX: ToolSpec = maintenance_tool("pg_reindex", "Rebuild indexes", false);
pub const PG_REFRESH: ToolSpec =
    maintenance_tool("pg_refresh", "Refresh a materialized view", false);
pub const PG_VACUUM_NEEDS: ToolSpec = ToolSpec {
    name: "pg_vacuum_needs",
    title: "Report tables that need a vacuum",
    group: Some(ToolGroup::Maintenance),
    modes: WRITE_MODES,
    scope: SCOPE_MAINTENANCE,
    read_only: true,
    destructive: false,
    idempotent: true,
};
pub const PG_BACKEND: ToolSpec =
    maintenance_tool("pg_backend", "Cancel or terminate a backend", true);
pub const PG_ACTIVITY: ToolSpec = monitoring_tool("pg_activity", "List running sessions");
pub const PG_LOCKS: ToolSpec = monitoring_tool("pg_locks", "List lock waits and blockers");
pub const PG_REPLICATION: ToolSpec = monitoring_tool("pg_replication", "Report replication state");
pub const PG_WAL: ToolSpec = monitoring_tool("pg_wal", "Report WAL and checkpoint activity");
pub const PG_INDEXES_HEALTH: ToolSpec = monitoring_tool(
    "pg_indexes_health",
    "Find invalid, duplicate, and unused indexes",
);
pub const PG_BLOAT: ToolSpec = monitoring_tool("pg_bloat", "Estimate table and index bloat");
pub const PG_SETTINGS: ToolSpec = monitoring_tool("pg_settings", "List server settings");
pub const PG_TOP_QUERIES: ToolSpec =
    monitoring_tool("pg_top_queries", "List the most expensive statements");

pub const TOOLS: &[ToolSpec] = &[
    PG_LIST_OBJECTS,
    PG_DESCRIBE,
    PG_RUN_QUERY,
    PG_COUNT,
    PG_EXPLAIN,
    PG_HEALTH,
    PG_DOCTOR,
    PG_INSERT,
    PG_UPDATE,
    PG_DELETE,
    PG_MERGE,
    PG_RUN_WRITE,
    PG_COPY,
    PG_TRANSACTION,
    PG_TABLE,
    PG_COLUMN,
    PG_CONSTRAINT,
    PG_INDEX,
    PG_VIEW,
    PG_SEQUENCE,
    PG_ROUTINE,
    PG_TRIGGER,
    PG_TYPE,
    PG_EXTENSION,
    PG_COMMENT,
    PG_ROLE,
    PG_GRANT,
    PG_POLICY,
    PG_PRIVILEGES,
    PG_VACUUM,
    PG_ANALYZE,
    PG_REINDEX,
    PG_REFRESH,
    PG_VACUUM_NEEDS,
    PG_BACKEND,
    PG_ACTIVITY,
    PG_LOCKS,
    PG_REPLICATION,
    PG_WAL,
    PG_INDEXES_HEALTH,
    PG_BLOAT,
    PG_SETTINGS,
    PG_TOP_QUERIES,
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

#[cfg(test)]
mod snapshots {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::{AppPaths, Environment, FlagLayer, Sources, resolve};
    use crate::tools::all_routes;

    pub(crate) const TOKEN_BUDGET_BYTES: usize = 24_000;

    fn settings_for(mode: Mode, tools: &str) -> Settings {
        let dir = tempfile::tempdir().unwrap();
        let mut vars = BTreeMap::from([("OWNPG_DATABASE".to_owned(), "app".to_owned())]);
        if !tools.is_empty() {
            vars.insert("OWNPG_TOOLS".to_owned(), tools.to_owned());
        }
        let env = Environment::new(vars, Some(dir.path().to_path_buf()), None);
        let paths = AppPaths::from_base(
            dir.path().join("c"),
            dir.path().join("d"),
            dir.path().join("k"),
        );
        resolve(
            FlagLayer {
                mode: Some(mode),
                ..FlagLayer::default()
            },
            Sources {
                env: &env,
                paths,
                keychain: None,
            },
        )
        .unwrap()
        .0
    }

    fn listed(settings: &Settings) -> Vec<rmcp::model::Tool> {
        let routes = all_routes().unwrap();
        loaded(settings)
            .iter()
            .filter_map(|spec| routes.iter().find(|route| route.spec.name == spec.name))
            .map(|route| route.tool.clone())
            .collect()
    }

    #[test]
    fn the_tool_list_is_stable_per_mode() {
        for (mode, label) in [
            (Mode::ReadOnly, "read-only"),
            (Mode::WriteOnly, "write-only"),
            (Mode::ReadWrite, "read-write"),
        ] {
            let tools = listed(&settings_for(mode, ""));
            insta::assert_json_snapshot!(format!("tools-list-{label}"), tools);
        }
    }

    #[test]
    fn the_default_set_fits_the_token_budget() {
        let tools = listed(&settings_for(Mode::ReadOnly, ""));
        let bytes = serde_json::to_string(&tools).unwrap().len();
        assert!(
            bytes <= TOKEN_BUDGET_BYTES,
            "the default tools/list is {bytes} bytes; the budget is {TOKEN_BUDGET_BYTES} bytes, four per token for 6000 tokens"
        );
    }
}
