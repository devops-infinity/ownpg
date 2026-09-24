use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Missing, Toggle, cascade_suffix, decimal, if_exists_clause, run_ddl, scoped_name};
use crate::error::{Error, Result};
use crate::render::{
    expression, quote_ident, quote_literal, returns_type, type_name, validate_ident,
};
use crate::tool_specs;
use crate::tools::{Call, Outcome, Route, route};

const ROUTINE_DESCRIPTION: &str = "Create, replace, alter, rename, or drop a function or procedure in the scoped schema. create takes typed arguments, the return type, the language (sql or plpgsql, the only two allowed), the body, volatility, strictness, parallel safety, and security. A security definer routine gets search_path pinned to the scoped schema and pg_temp. drop is destructive and needs confirm: true or the confirmation prompt.";

const ALLOWED_LANGUAGES: [&str; 2] = ["sql", "plpgsql"];

fn safe_language(argument: &str, text: &str) -> Result<String> {
    let trimmed = text.trim();
    validate_ident(argument, trimmed)?;
    let lowered = trimmed.to_ascii_lowercase();
    if !ALLOWED_LANGUAGES.contains(&lowered.as_str()) {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!(
                "`{trimmed}` is not an allowed language; only sql and plpgsql are permitted, \
                 since any other procedural language runs with the interpreter's own \
                 capabilities, not the classifier's"
            ),
        });
    }
    Ok(lowered)
}

const TRIGGER_DESCRIPTION: &str = "Create, drop, rename, enable, or disable a trigger on a table in the scoped schema, or create and drop an event trigger. create takes the timing, the events, the row or statement level, an optional WHEN condition, transition table names, and the trigger function with its arguments. drop is destructive and needs confirm: true or the confirmation prompt.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutineOperation {
    Create,
    Alter,
    Rename,
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutineKind {
    #[default]
    Function,
    Procedure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentMode {
    #[default]
    In,
    Out,
    Inout,
    Variadic,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    #[serde(default)]
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub mode: ArgumentMode,
    #[serde(default)]
    pub default: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Volatility {
    #[default]
    Unset,
    Volatile,
    Stable,
    Immutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Parallel {
    #[default]
    Unset,
    Unsafe,
    Restricted,
    Safe,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineArgs {
    pub operation: RoutineOperation,
    #[schemars(description = "Routine name, optionally schema-qualified.")]
    pub name: String,
    #[serde(default)]
    pub kind: RoutineKind,
    #[serde(default)]
    #[schemars(
        description = "The arguments. For alter, rename, and drop, the types identify the overload."
    )]
    pub arguments: Vec<Argument>,
    #[serde(default)]
    #[schemars(
        description = "create function: the return type, such as integer, SETOF text, TABLE (id int), or trigger."
    )]
    pub returns: String,
    #[serde(default)]
    #[schemars(description = "create: the language, such as sql or plpgsql.")]
    pub language: String,
    #[serde(default)]
    #[schemars(description = "create: the body.")]
    pub body: String,
    #[serde(default)]
    pub or_replace: bool,
    #[serde(default)]
    pub volatility: Volatility,
    #[serde(default)]
    #[schemars(
        description = "create or alter: on returns NULL on NULL input; off calls the routine anyway."
    )]
    pub strict: Toggle,
    #[serde(default)]
    #[schemars(
        description = "create or alter: on runs with the owner's privileges and pins search_path."
    )]
    pub security_definer: Toggle,
    #[serde(default)]
    pub parallel: Parallel,
    #[serde(default)]
    #[schemars(description = "Planner cost estimate as text; empty leaves it unset.")]
    pub cost: String,
    #[serde(default)]
    #[schemars(
        description = "Planner row estimate as text for set-returning functions; empty leaves it unset."
    )]
    pub row_estimate: String,
    #[serde(default)]
    #[schemars(
        description = "create or alter: settings such as \"work_mem = '64MB'\" applied while the routine runs."
    )]
    pub set_config: Vec<String>,
    #[serde(default)]
    #[schemars(description = "alter: the new owner.")]
    pub owner: String,
    #[serde(default)]
    #[schemars(description = "rename: the new name.")]
    pub new_name: String,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

fn argument_list(arguments: &[Argument], with_defaults: bool) -> Result<String> {
    let mut parts = Vec::new();
    for argument in arguments {
        let mut part = String::new();
        match argument.mode {
            ArgumentMode::In => {}
            ArgumentMode::Out => part.push_str("OUT "),
            ArgumentMode::Inout => part.push_str("INOUT "),
            ArgumentMode::Variadic => part.push_str("VARIADIC "),
        }
        if !argument.name.trim().is_empty() {
            validate_ident("arguments", argument.name.trim())?;
            part.push_str(&quote_ident(argument.name.trim()));
            part.push(' ');
        }
        part.push_str(&type_name("arguments", &argument.data_type)?);
        if with_defaults && !argument.default.trim().is_empty() {
            part.push_str(&format!(
                " DEFAULT {}",
                expression("arguments", &argument.default)?
            ));
        }
        parts.push(part);
    }
    Ok(parts.join(", "))
}

fn signature_types(arguments: &[Argument]) -> Result<String> {
    let mut parts = Vec::new();
    for argument in arguments {
        if matches!(argument.mode, ArgumentMode::Out) {
            continue;
        }
        parts.push(type_name("arguments", &argument.data_type)?);
    }
    Ok(parts.join(", "))
}

#[must_use]
pub fn dollar_quote(body: &str) -> String {
    let mut tag = "ownpg".to_owned();
    let mut counter = 0u32;
    while body.contains(&format!("${tag}$")) {
        counter += 1;
        tag = format!("ownpg{counter}");
    }
    format!("${tag}${body}${tag}$")
}

fn config_clauses(field: &str, settings: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for setting in settings {
        let (name, value) = setting
            .split_once('=')
            .ok_or_else(|| Error::ArgumentInvalid {
                argument: field.to_owned(),
                detail: format!("`{setting}` is not name = value"),
            })?;
        validate_ident(field, name.trim())?;
        let value = value.trim();
        if value.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: field.to_owned(),
                detail: format!("`{setting}` has no value"),
            });
        }
        let rendered = if value.starts_with('\'') {
            expression(field, value)?
        } else if value
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            value.to_owned()
        } else {
            quote_literal(value)
        };
        out.push(format!("SET {} = {rendered}", quote_ident(name.trim())));
    }
    Ok(out)
}

fn definer_search_path(schema: &str, set_config: &[String]) -> Result<String> {
    let overrides_path = set_config.iter().any(|setting| {
        setting
            .split_once('=')
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("search_path"))
    });
    if overrides_path {
        return Err(Error::ArgumentInvalid {
            argument: "set_config".to_owned(),
            detail: format!(
                "a SECURITY DEFINER routine keeps the pinned search_path `{schema}, pg_temp`, so a caller cannot shadow the objects it uses; remove search_path from set_config"
            ),
        });
    }
    Ok(format!(
        "SET search_path = {}, pg_temp",
        quote_ident(schema)
    ))
}

pub fn routine(call: Call, args: RoutineArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let name = scoped_name(&call, "name", &args.name)?;
        let word = match args.kind {
            RoutineKind::Function => "FUNCTION",
            RoutineKind::Procedure => "PROCEDURE",
        };
        let signature = format!("{}({})", name.sql(), signature_types(&args.arguments)?);
        let (sql, kinds): (String, &[&str]) = match args.operation {
            RoutineOperation::Create => {
                let mut missing = Missing::new();
                missing
                    .need("language", !args.language.trim().is_empty())
                    .need("body", !args.body.trim().is_empty())
                    .need(
                        "returns",
                        args.kind == RoutineKind::Procedure || !args.returns.trim().is_empty(),
                    );
                missing.finish("create")?;
                let language = safe_language("language", &args.language)?;
                let mut sql = format!(
                    "CREATE{} {word} {}({})",
                    if args.or_replace { " OR REPLACE" } else { "" },
                    name.sql(),
                    argument_list(&args.arguments, true)?
                );
                if args.kind == RoutineKind::Function {
                    sql.push_str(&format!(
                        " RETURNS {}",
                        returns_type("returns", &args.returns)?
                    ));
                }
                sql.push_str(&format!(" LANGUAGE {}", quote_ident(&language)));
                sql.push_str(match args.volatility {
                    Volatility::Unset => "",
                    Volatility::Volatile => " VOLATILE",
                    Volatility::Stable => " STABLE",
                    Volatility::Immutable => " IMMUTABLE",
                });
                if args.strict == Toggle::On {
                    sql.push_str(" STRICT");
                }
                sql.push_str(match args.parallel {
                    Parallel::Unset => "",
                    Parallel::Unsafe => " PARALLEL UNSAFE",
                    Parallel::Restricted => " PARALLEL RESTRICTED",
                    Parallel::Safe => " PARALLEL SAFE",
                });
                if let Some(cost) = decimal("cost", &args.cost)? {
                    sql.push_str(&format!(" COST {cost}"));
                }
                if let Some(rows) = decimal("row_estimate", &args.row_estimate)? {
                    sql.push_str(&format!(" ROWS {rows}"));
                }
                let mut settings = config_clauses("set_config", &args.set_config)?;
                if args.security_definer == Toggle::On {
                    sql.push_str(" SECURITY DEFINER");
                    settings.insert(
                        0,
                        definer_search_path(&call.settings().schema.value, &args.set_config)?,
                    );
                }
                for setting in settings {
                    sql.push_str(&format!(" {setting}"));
                }
                sql.push_str(&format!(" AS {}", dollar_quote(&args.body)));
                (sql, &["CreateFunctionStmt"])
            }
            RoutineOperation::Alter => {
                let mut clauses = Vec::new();
                match args.volatility {
                    Volatility::Unset => {}
                    Volatility::Volatile => clauses.push("VOLATILE".to_owned()),
                    Volatility::Stable => clauses.push("STABLE".to_owned()),
                    Volatility::Immutable => clauses.push("IMMUTABLE".to_owned()),
                }
                if let Some(strict) = args.strict.as_bool() {
                    clauses.push(
                        if strict {
                            "STRICT"
                        } else {
                            "CALLED ON NULL INPUT"
                        }
                        .to_owned(),
                    );
                }
                if let Some(definer) = args.security_definer.as_bool() {
                    clauses.push(
                        if definer {
                            "SECURITY DEFINER"
                        } else {
                            "SECURITY INVOKER"
                        }
                        .to_owned(),
                    );
                }
                match args.parallel {
                    Parallel::Unset => {}
                    Parallel::Unsafe => clauses.push("PARALLEL UNSAFE".to_owned()),
                    Parallel::Restricted => clauses.push("PARALLEL RESTRICTED".to_owned()),
                    Parallel::Safe => clauses.push("PARALLEL SAFE".to_owned()),
                }
                if let Some(cost) = decimal("cost", &args.cost)? {
                    clauses.push(format!("COST {cost}"));
                }
                if let Some(rows) = decimal("row_estimate", &args.row_estimate)? {
                    clauses.push(format!("ROWS {rows}"));
                }
                clauses.extend(config_clauses("set_config", &args.set_config)?);
                if args.security_definer == Toggle::On {
                    clauses.push(definer_search_path(
                        &call.settings().schema.value,
                        &args.set_config,
                    )?);
                }
                if !args.owner.trim().is_empty() {
                    validate_ident("owner", args.owner.trim())?;
                    if !clauses.is_empty() {
                        return Err(Error::ArgumentInvalid {
                            argument: "owner".to_owned(),
                            detail: "an owner change is its own alter call".to_owned(),
                        }
                        .into());
                    }
                    (
                        format!(
                            "ALTER {word} {signature} OWNER TO {}",
                            quote_ident(args.owner.trim())
                        ),
                        &["AlterOwnerStmt"],
                    )
                } else {
                    if clauses.is_empty() {
                        return Err(Error::ArgumentInvalid {
                            argument: "operation".to_owned(),
                            detail: "alter needs at least one property to change".to_owned(),
                        }
                        .into());
                    }
                    (
                        format!("ALTER {word} {signature} {}", clauses.join(" ")),
                        &["AlterFunctionStmt"],
                    )
                }
            }
            RoutineOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER {word} {signature} RENAME TO {}",
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            RoutineOperation::Drop => (
                format!(
                    "DROP {word}{} {signature}{}",
                    if_exists_clause(args.if_exists),
                    cascade_suffix(args.cascade)
                ),
                &["DropStmt"],
            ),
        };
        run_ddl(
            &call,
            "pg_routine",
            sql,
            kinds,
            args.dry_run,
            args.confirm,
            &args.transaction,
        )
        .await
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriggerOperation {
    Create,
    Drop,
    Rename,
    Enable,
    Disable,
    CreateEvent,
    DropEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Timing {
    #[default]
    Unset,
    Before,
    After,
    InsteadOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
    Truncate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnableMode {
    #[default]
    Origin,
    Replica,
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriggerArgs {
    pub operation: TriggerOperation,
    #[schemars(description = "Trigger name.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "The table, optionally schema-qualified (not for event triggers).")]
    pub table: String,
    #[serde(default)]
    #[schemars(description = "create: before, after, or instead_of.")]
    pub timing: Timing,
    #[serde(default)]
    pub events: Vec<TriggerEvent>,
    #[serde(default)]
    #[schemars(description = "create: fire only when these columns change (UPDATE OF).")]
    pub update_columns: Vec<String>,
    #[serde(default)]
    #[schemars(description = "create: fire once per row instead of once per statement.")]
    pub for_each_row: bool,
    #[serde(default)]
    #[schemars(description = "create: WHEN condition over OLD and NEW.")]
    pub when: String,
    #[serde(default)]
    #[schemars(description = "create: the trigger function, optionally schema-qualified.")]
    pub function: String,
    #[serde(default)]
    #[schemars(description = "create: literal arguments passed to the function.")]
    pub arguments: Vec<String>,
    #[serde(default)]
    #[schemars(description = "create: transition table name for OLD TABLE.")]
    pub old_table: String,
    #[serde(default)]
    #[schemars(description = "create: transition table name for NEW TABLE.")]
    pub new_table: String,
    #[serde(default)]
    pub or_replace: bool,
    #[serde(default)]
    #[schemars(description = "create: a constraint trigger that can be deferred.")]
    pub constraint_trigger: bool,
    #[serde(default)]
    pub deferrable: bool,
    #[serde(default)]
    pub initially_deferred: bool,
    #[serde(default)]
    #[schemars(description = "enable: origin (default), replica, or always.")]
    pub enable_mode: EnableMode,
    #[serde(default)]
    #[schemars(
        description = "create_event: ddl_command_start, ddl_command_end, table_rewrite, or sql_drop."
    )]
    pub event: String,
    #[serde(default)]
    #[schemars(
        description = "create_event: command tags that fire the trigger, such as CREATE TABLE."
    )]
    pub tags: Vec<String>,
    #[serde(default)]
    #[schemars(description = "rename: the new name.")]
    pub new_name: String,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

pub fn trigger(call: Call, args: TriggerArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        validate_ident("name", &args.name)?;
        let trigger_name = quote_ident(&args.name);
        let on_table = |field: &str| -> Result<String> {
            let mut missing = Missing::new();
            missing.need("table", !args.table.trim().is_empty());
            missing.finish(field)?;
            Ok(scoped_name(&call, "table", &args.table)?.sql())
        };
        let (sql, kinds): (String, &[&str]) = match args.operation {
            TriggerOperation::Create => {
                let table = on_table("create")?;
                let mut missing = Missing::new();
                missing
                    .need("timing", args.timing != Timing::Unset)
                    .need("events", !args.events.is_empty())
                    .need("function", !args.function.trim().is_empty());
                missing.finish("create")?;
                let timing = match args.timing {
                    Timing::Before => "BEFORE",
                    Timing::Unset | Timing::After => "AFTER",
                    Timing::InsteadOf => "INSTEAD OF",
                };
                let mut events = Vec::new();
                for event in &args.events {
                    let mut text = match event {
                        TriggerEvent::Insert => "INSERT".to_owned(),
                        TriggerEvent::Update => "UPDATE".to_owned(),
                        TriggerEvent::Delete => "DELETE".to_owned(),
                        TriggerEvent::Truncate => "TRUNCATE".to_owned(),
                    };
                    if *event == TriggerEvent::Update && !args.update_columns.is_empty() {
                        text.push_str(&format!(
                            " OF {}",
                            crate::render::ident_list("update_columns", &args.update_columns)?
                        ));
                    }
                    events.push(text);
                }
                let function = scoped_name(&call, "function", &args.function)?;
                let literals: Vec<String> =
                    args.arguments.iter().map(|a| quote_literal(a)).collect();
                let mut sql = format!(
                    "CREATE{}{} TRIGGER {trigger_name} {timing} {} ON {table}",
                    if args.or_replace { " OR REPLACE" } else { "" },
                    if args.constraint_trigger {
                        " CONSTRAINT"
                    } else {
                        ""
                    },
                    events.join(" OR ")
                );
                if args.constraint_trigger && args.deferrable {
                    sql.push_str(" DEFERRABLE");
                    if args.initially_deferred {
                        sql.push_str(" INITIALLY DEFERRED");
                    }
                }
                let mut transitions = Vec::new();
                if !args.old_table.trim().is_empty() {
                    validate_ident("old_table", args.old_table.trim())?;
                    transitions.push(format!(
                        "OLD TABLE AS {}",
                        quote_ident(args.old_table.trim())
                    ));
                }
                if !args.new_table.trim().is_empty() {
                    validate_ident("new_table", args.new_table.trim())?;
                    transitions.push(format!(
                        "NEW TABLE AS {}",
                        quote_ident(args.new_table.trim())
                    ));
                }
                if !transitions.is_empty() {
                    sql.push_str(&format!(" REFERENCING {}", transitions.join(" ")));
                }
                sql.push_str(if args.for_each_row {
                    " FOR EACH ROW"
                } else {
                    " FOR EACH STATEMENT"
                });
                if !args.when.trim().is_empty() {
                    sql.push_str(&format!(" WHEN ({})", expression("when", &args.when)?));
                }
                sql.push_str(&format!(
                    " EXECUTE FUNCTION {}({})",
                    function.sql(),
                    literals.join(", ")
                ));
                (sql, &["CreateTrigStmt"])
            }
            TriggerOperation::Drop => {
                let table = on_table("drop")?;
                (
                    format!(
                        "DROP TRIGGER{} {trigger_name} ON {table}{}",
                        if_exists_clause(args.if_exists),
                        cascade_suffix(args.cascade)
                    ),
                    &["DropStmt"],
                )
            }
            TriggerOperation::Rename => {
                let table = on_table("rename")?;
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER TRIGGER {trigger_name} ON {table} RENAME TO {}",
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            TriggerOperation::Enable => {
                let table = on_table("enable")?;
                let mode = match args.enable_mode {
                    EnableMode::Origin => "ENABLE",
                    EnableMode::Replica => "ENABLE REPLICA",
                    EnableMode::Always => "ENABLE ALWAYS",
                };
                (
                    format!("ALTER TABLE {table} {mode} TRIGGER {trigger_name}"),
                    &["AlterTableStmt"],
                )
            }
            TriggerOperation::Disable => {
                let table = on_table("disable")?;
                (
                    format!("ALTER TABLE {table} DISABLE TRIGGER {trigger_name}"),
                    &["AlterTableStmt"],
                )
            }
            TriggerOperation::CreateEvent => {
                let mut missing = Missing::new();
                missing
                    .need("event", !args.event.trim().is_empty())
                    .need("function", !args.function.trim().is_empty());
                missing.finish("create_event")?;
                let event = args.event.trim().to_ascii_lowercase();
                if ![
                    "ddl_command_start",
                    "ddl_command_end",
                    "table_rewrite",
                    "sql_drop",
                ]
                .contains(&event.as_str())
                {
                    return Err(Error::ArgumentInvalid {
                        argument: "event".to_owned(),
                        detail: "event is ddl_command_start, ddl_command_end, table_rewrite, or sql_drop".to_owned(),
                    }
                    .into());
                }
                let function = scoped_name(&call, "function", &args.function)?;
                let mut sql = format!("CREATE EVENT TRIGGER {trigger_name} ON {event}");
                if !args.tags.is_empty() {
                    let tags: Vec<String> = args.tags.iter().map(|t| quote_literal(t)).collect();
                    sql.push_str(&format!(" WHEN TAG IN ({})", tags.join(", ")));
                }
                sql.push_str(&format!(" EXECUTE FUNCTION {}()", function.sql()));
                (sql, &["CreateEventTrigStmt"])
            }
            TriggerOperation::DropEvent => (
                format!(
                    "DROP EVENT TRIGGER{} {trigger_name}{}",
                    if_exists_clause(args.if_exists),
                    cascade_suffix(args.cascade)
                ),
                &["DropStmt"],
            ),
        };
        run_ddl(
            &call,
            "pg_trigger",
            sql,
            kinds,
            args.dry_run,
            args.confirm,
            &args.transaction,
        )
        .await
    })
}

pub fn routes() -> Result<Vec<Route>> {
    Ok(vec![
        route::<RoutineArgs, crate::tools::write::StatementOutput, _>(
            &tool_specs::PG_ROUTINE,
            ROUTINE_DESCRIPTION,
            routine,
        )?,
        route::<TriggerArgs, crate::tools::write::StatementOutput, _>(
            &tool_specs::PG_TRIGGER,
            TRIGGER_DESCRIPTION,
            trigger,
        )?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollar_quoting_picks_a_tag_absent_from_the_body() {
        assert_eq!(dollar_quote("SELECT 1"), "$ownpg$SELECT 1$ownpg$");
        assert_eq!(
            dollar_quote("SELECT '$ownpg$'"),
            "$ownpg1$SELECT '$ownpg$'$ownpg1$"
        );
    }

    #[test]
    fn config_clauses_quote_values_and_refuse_junk() {
        let clauses = config_clauses(
            "set_config",
            &[
                "work_mem = '64MB'".to_owned(),
                "search_path = app".to_owned(),
            ],
        )
        .unwrap();
        assert_eq!(clauses[0], "SET \"work_mem\" = '64MB'");
        assert_eq!(clauses[1], "SET \"search_path\" = app");
        assert!(config_clauses("set_config", &["nonsense".to_owned()]).is_err());
        let quoted = config_clauses("set_config", &["x = 1; DROP TABLE t".to_owned()]).unwrap();
        assert_eq!(quoted[0], "SET \"x\" = '1; DROP TABLE t'");
    }

    #[test]
    fn a_security_definer_routine_pins_its_search_path() {
        assert_eq!(
            definer_search_path("App", &["work_mem = 64MB".to_owned()]).unwrap(),
            "SET search_path = \"App\", pg_temp"
        );
        for set_config in [
            ["search_path = app".to_owned()],
            ["SEARCH_PATH=public".to_owned()],
        ] {
            let error = definer_search_path("app", &set_config).unwrap_err();
            assert!(error.to_string().contains("search_path"), "{error}");
        }
    }

    #[test]
    fn signatures_skip_out_arguments() {
        let arguments = vec![
            Argument {
                name: "a".to_owned(),
                data_type: "int".to_owned(),
                mode: ArgumentMode::In,
                default: String::new(),
            },
            Argument {
                name: "b".to_owned(),
                data_type: "text".to_owned(),
                mode: ArgumentMode::Out,
                default: String::new(),
            },
        ];
        assert_eq!(signature_types(&arguments).unwrap(), "int");
        assert_eq!(
            argument_list(&arguments, true).unwrap(),
            "\"a\" int, OUT \"b\" text"
        );
    }

    #[test]
    fn only_sql_and_plpgsql_are_allowed_languages() {
        assert_eq!(safe_language("language", "sql").unwrap(), "sql");
        assert_eq!(safe_language("language", "PlPgSQL").unwrap(), "plpgsql");
        for unsafe_language in ["plpythonu", "plperlu", "c", "plv8", "sh"] {
            assert!(
                safe_language("language", unsafe_language).is_err(),
                "{unsafe_language} should not be an allowed language"
            );
        }
    }
}
