use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    Missing, Toggle, cascade_suffix, if_exists_clause, if_not_exists_clause, number, run_ddl,
    scoped_name,
};
use crate::error::{Error, Result};
use crate::render::{expression, ident_list, quote_ident, type_name, validate_ident};
use crate::tool_specs;
use crate::tools::read::classify_checked;
use crate::tools::{Call, Outcome, Route, route};

const INDEX_DESCRIPTION: &str = "Create, drop, rename, or rebuild an index on a table in the scoped schema. create takes columns or expressions (with an optional DESC or NULLS FIRST suffix), a method (btree, hash, gist, spgist, gin, brin), unique, include columns, a WHERE predicate, and concurrently, which builds without blocking writes and runs outside any transaction handle. An index left INVALID by a failed concurrent build shows in pg_describe; reindex it or drop it and build again. drop is destructive and needs confirm: true or the confirmation prompt.";

const VIEW_DESCRIPTION: &str = "Create, replace, drop, rename, or refresh a view or materialized view in the scoped schema. create takes a SELECT, which is classified as a read first. Views can carry a check option and security_invoker; materialized views can be created without data and refreshed later, concurrently when a unique index exists. drop is destructive and needs confirm: true or the confirmation prompt.";

const SEQUENCE_DESCRIPTION: &str = "Create, alter, rename, or drop a sequence in the scoped schema, with type, increment, minimum, maximum, start, cache, cycle, restart, and the owning column. drop is destructive and needs confirm: true or the confirmation prompt.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexOperation {
    Create,
    Drop,
    Rename,
    Reindex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexMethod {
    #[default]
    Btree,
    Hash,
    Gist,
    Spgist,
    Gin,
    Brin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReindexTarget {
    #[default]
    Index,
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexArgs {
    pub operation: IndexOperation,
    #[serde(default)]
    #[schemars(description = "Index name. Optional for create; PostgreSQL names it otherwise.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "create and reindex table: the table.")]
    pub table: String,
    #[serde(default)]
    #[schemars(
        description = "create: columns or expressions, each optionally followed by DESC, ASC, NULLS FIRST, or NULLS LAST."
    )]
    pub columns: Vec<String>,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub concurrently: bool,
    #[serde(default)]
    pub method: IndexMethod,
    #[serde(default)]
    #[schemars(description = "create: non-key columns to include.")]
    pub include: Vec<String>,
    #[serde(default)]
    #[schemars(description = "create: predicate for a partial index.")]
    pub where_clause: String,
    #[serde(default)]
    #[schemars(description = "create unique: NULLS NOT DISTINCT (PostgreSQL 15 and later).")]
    pub nulls_not_distinct: bool,
    #[serde(default)]
    pub if_not_exists: bool,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    #[schemars(description = "rename: the new index name.")]
    pub new_name: String,
    #[serde(default)]
    #[schemars(description = "reindex: index (default) or table.")]
    pub target: ReindexTarget,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

pub(crate) fn index_element(element: &str) -> Result<String> {
    let trimmed = element.trim();
    let lowered = trimmed.to_ascii_lowercase();
    let suffixes = [
        " desc nulls first",
        " desc nulls last",
        " asc nulls first",
        " asc nulls last",
        " nulls first",
        " nulls last",
        " desc",
        " asc",
    ];
    let (body, suffix) = suffixes
        .iter()
        .find(|suffix| lowered.ends_with(*suffix))
        .map_or((trimmed, ""), |suffix| {
            (
                trimmed
                    .get(..trimmed.len() - suffix.len())
                    .unwrap_or(trimmed),
                *suffix,
            )
        });
    let body = body.trim();
    let rendered = if body.chars().all(|c| c.is_alphanumeric() || c == '_') {
        validate_ident("columns", body)?;
        quote_ident(body)
    } else {
        format!("({})", expression("columns", body)?)
    };
    Ok(format!("{rendered}{}", suffix.to_ascii_uppercase()))
}

pub fn index(call: Call, args: IndexArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let (sql, kinds): (String, &[&str]) = match args.operation {
            IndexOperation::Create => {
                let mut missing = Missing::new();
                missing
                    .need("table", !args.table.trim().is_empty())
                    .need("columns", !args.columns.is_empty());
                missing.finish("create")?;
                let table = scoped_name(&call, "table", &args.table)?;
                let elements: Result<Vec<String>> =
                    args.columns.iter().map(|c| index_element(c)).collect();
                let name = if args.name.trim().is_empty() {
                    if args.if_not_exists {
                        return Err(Error::ArgumentInvalid {
                            argument: "if_not_exists".to_owned(),
                            detail: "if_not_exists needs the index name, because PostgreSQL can only skip an index it can find by name".to_owned(),
                        }
                        .into());
                    }
                    String::new()
                } else {
                    validate_ident("name", args.name.trim())?;
                    format!(" {}", quote_ident(args.name.trim()))
                };
                let method = match args.method {
                    IndexMethod::Btree => "btree",
                    IndexMethod::Hash => "hash",
                    IndexMethod::Gist => "gist",
                    IndexMethod::Spgist => "spgist",
                    IndexMethod::Gin => "gin",
                    IndexMethod::Brin => "brin",
                };
                let mut sql = format!(
                    "CREATE{} INDEX{}{}{} ON {} USING {method} ({})",
                    if args.unique { " UNIQUE" } else { "" },
                    if args.concurrently {
                        " CONCURRENTLY"
                    } else {
                        ""
                    },
                    if_not_exists_clause(args.if_not_exists),
                    name,
                    table.sql(),
                    elements?.join(", ")
                );
                if !args.include.is_empty() {
                    sql.push_str(&format!(
                        " INCLUDE ({})",
                        ident_list("include", &args.include)?
                    ));
                }
                if args.nulls_not_distinct {
                    if !call.engine().features().supports_nulls_not_distinct() {
                        return Err(Error::ArgumentInvalid {
                            argument: "nulls_not_distinct".to_owned(),
                            detail: "NULLS NOT DISTINCT needs PostgreSQL 15 or later".to_owned(),
                        }
                        .into());
                    }
                    sql.push_str(" NULLS NOT DISTINCT");
                }
                if !args.where_clause.trim().is_empty() {
                    sql.push_str(&format!(
                        " WHERE {}",
                        expression("where_clause", &args.where_clause)?
                    ));
                }
                (sql, &["IndexStmt"])
            }
            IndexOperation::Drop => {
                let name = scoped_name(&call, "name", &args.name)?;
                (
                    format!(
                        "DROP INDEX{}{} {}{}",
                        if args.concurrently {
                            " CONCURRENTLY"
                        } else {
                            ""
                        },
                        if_exists_clause(args.if_exists),
                        name.sql(),
                        cascade_suffix(args.cascade)
                    ),
                    &["DropStmt"],
                )
            }
            IndexOperation::Rename => {
                let name = scoped_name(&call, "name", &args.name)?;
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER INDEX {} RENAME TO {}",
                        name.sql(),
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            IndexOperation::Reindex => {
                let (word, name) = match args.target {
                    ReindexTarget::Index => ("INDEX", scoped_name(&call, "name", &args.name)?),
                    ReindexTarget::Table => ("TABLE", scoped_name(&call, "table", &args.table)?),
                };
                (
                    format!(
                        "REINDEX {word}{} {}",
                        if args.concurrently {
                            " CONCURRENTLY"
                        } else {
                            ""
                        },
                        name.sql()
                    ),
                    &["ReindexStmt"],
                )
            }
        };
        run_ddl(
            &call,
            "pg_index",
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
pub enum ViewOperation {
    Create,
    Drop,
    Rename,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckOption {
    #[default]
    None,
    Local,
    Cascaded,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewArgs {
    pub operation: ViewOperation,
    #[schemars(description = "View name, optionally schema-qualified.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "The view is a materialized view.")]
    pub materialized: bool,
    #[serde(default)]
    #[schemars(description = "create: the SELECT that defines the view.")]
    pub query: String,
    #[serde(default)]
    #[schemars(description = "create: column names for the view.")]
    pub columns: Vec<String>,
    #[serde(default)]
    pub or_replace: bool,
    #[serde(default)]
    pub if_not_exists: bool,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    #[schemars(description = "create view: WITH CHECK OPTION, local or cascaded.")]
    pub check_option: CheckOption,
    #[serde(default)]
    #[schemars(
        description = "create view: run with the caller's privileges (PostgreSQL 15 and later)."
    )]
    pub security_invoker: bool,
    #[serde(default = "default_true")]
    #[schemars(description = "create and refresh materialized: populate the view (default true).")]
    pub with_data: bool,
    #[serde(default)]
    #[schemars(description = "refresh: refresh concurrently, which needs a unique index.")]
    pub concurrently: bool,
    #[serde(default)]
    #[schemars(description = "rename: the new name.")]
    pub new_name: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

const fn default_true() -> bool {
    true
}

pub fn view(call: Call, args: ViewArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let name = scoped_name(&call, "name", &args.name)?;
        let word = if args.materialized {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };
        let (sql, kinds): (String, &[&str]) = match args.operation {
            ViewOperation::Create => {
                let mut missing = Missing::new();
                missing.need("query", !args.query.trim().is_empty());
                missing.finish("create")?;
                let inner = classify_checked(&call, args.query.trim()).await?;
                if inner.kind != "SelectStmt" {
                    return Err(Error::StatementRefused {
                        rule: "a view is defined by a SELECT".to_owned(),
                        mode: call.settings().mode.value.to_string(),
                    }
                    .into());
                }
                let columns = if args.columns.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", ident_list("columns", &args.columns)?)
                };
                let mut sql = if args.materialized {
                    format!(
                        "CREATE MATERIALIZED VIEW{} {}{} AS {}\n{}",
                        if_not_exists_clause(args.if_not_exists),
                        name.sql(),
                        columns,
                        args.query.trim(),
                        if args.with_data {
                            "WITH DATA"
                        } else {
                            "WITH NO DATA"
                        }
                    )
                } else {
                    let options = if args.security_invoker {
                        if !call.engine().features().supports_security_invoker() {
                            return Err(Error::ArgumentInvalid {
                                argument: "security_invoker".to_owned(),
                                detail: "security_invoker needs PostgreSQL 15 or later".to_owned(),
                            }
                            .into());
                        }
                        " WITH (security_invoker = true)"
                    } else {
                        ""
                    };
                    format!(
                        "CREATE{} VIEW {}{}{} AS {}",
                        if args.or_replace { " OR REPLACE" } else { "" },
                        name.sql(),
                        columns,
                        options,
                        args.query.trim()
                    )
                };
                if args.check_option != CheckOption::None {
                    if args.materialized {
                        return Err(Error::ArgumentInvalid {
                            argument: "check_option".to_owned(),
                            detail: "a materialized view has no check option".to_owned(),
                        }
                        .into());
                    }
                    sql.push_str(match args.check_option {
                        CheckOption::None | CheckOption::Local => "\nWITH LOCAL CHECK OPTION",
                        CheckOption::Cascaded => "\nWITH CASCADED CHECK OPTION",
                    });
                }
                (sql, &["ViewStmt", "CreateTableAsStmt"])
            }
            ViewOperation::Drop => (
                format!(
                    "DROP {word}{} {}{}",
                    if_exists_clause(args.if_exists),
                    name.sql(),
                    cascade_suffix(args.cascade)
                ),
                &["DropStmt"],
            ),
            ViewOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER {word} {} RENAME TO {}",
                        name.sql(),
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            ViewOperation::Refresh => {
                if !args.materialized {
                    return Err(Error::ArgumentInvalid {
                        argument: "materialized".to_owned(),
                        detail: "only a materialized view is refreshed".to_owned(),
                    }
                    .into());
                }
                (
                    format!(
                        "REFRESH MATERIALIZED VIEW{} {}{}",
                        if args.concurrently {
                            " CONCURRENTLY"
                        } else {
                            ""
                        },
                        name.sql(),
                        if args.with_data {
                            " WITH DATA"
                        } else {
                            " WITH NO DATA"
                        }
                    ),
                    &["RefreshMatViewStmt"],
                )
            }
        };
        run_ddl(
            &call,
            "pg_view",
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
pub enum SequenceOperation {
    Create,
    Alter,
    Drop,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceArgs {
    pub operation: SequenceOperation,
    #[schemars(description = "Sequence name, optionally schema-qualified.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "create: smallint, integer, or bigint.")]
    pub as_type: String,
    #[serde(default)]
    #[schemars(description = "Whole number as text; empty leaves it unset.")]
    pub increment: String,
    #[serde(default)]
    #[schemars(description = "Whole number as text; empty leaves it unset.")]
    pub min_value: String,
    #[serde(default)]
    #[schemars(description = "Whole number as text; empty leaves it unset.")]
    pub max_value: String,
    #[serde(default)]
    #[schemars(description = "Whole number as text; empty leaves it unset.")]
    pub start: String,
    #[serde(default)]
    #[schemars(description = "alter: restart from this value.")]
    pub restart: String,
    #[serde(default)]
    #[schemars(description = "Whole number as text; empty leaves it unset.")]
    pub cache: String,
    #[serde(default)]
    pub cycle: Toggle,
    #[serde(default)]
    #[schemars(description = "table.column that owns the sequence, or NONE.")]
    pub owned_by: String,
    #[serde(default)]
    pub if_not_exists: bool,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    #[schemars(description = "rename: the new name.")]
    pub new_name: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

fn sequence_options(call: &Call, args: &SequenceArgs, altering: bool) -> Result<String> {
    let mut parts = Vec::new();
    if !args.as_type.trim().is_empty() {
        parts.push(format!("AS {}", type_name("as_type", &args.as_type)?));
    }
    if let Some(value) = number("increment", &args.increment)? {
        parts.push(format!("INCREMENT BY {value}"));
    }
    if let Some(value) = number("min_value", &args.min_value)? {
        parts.push(format!("MINVALUE {value}"));
    }
    if let Some(value) = number("max_value", &args.max_value)? {
        parts.push(format!("MAXVALUE {value}"));
    }
    if let Some(value) = number("start", &args.start)? {
        parts.push(format!("START WITH {value}"));
    }
    if altering && let Some(value) = number("restart", &args.restart)? {
        parts.push(format!("RESTART WITH {value}"));
    }
    if let Some(value) = number("cache", &args.cache)? {
        parts.push(format!("CACHE {value}"));
    }
    if let Some(cycle) = args.cycle.as_bool() {
        parts.push(if cycle { "CYCLE" } else { "NO CYCLE" }.to_owned());
    }
    let owner = args.owned_by.trim();
    if !owner.is_empty() {
        if owner.eq_ignore_ascii_case("none") {
            parts.push("OWNED BY NONE".to_owned());
        } else {
            let (table, column) = owner
                .rsplit_once('.')
                .ok_or_else(|| Error::ArgumentInvalid {
                    argument: "owned_by".to_owned(),
                    detail: "owned_by is table.column or NONE".to_owned(),
                })?;
            let table = scoped_name(call, "owned_by", table)?;
            validate_ident("owned_by", column)?;
            parts.push(format!("OWNED BY {}.{}", table.sql(), quote_ident(column)));
        }
    }
    Ok(parts.join(" "))
}

pub fn sequence(call: Call, args: SequenceArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let name = scoped_name(&call, "name", &args.name)?;
        let (sql, kinds): (String, &[&str]) = match args.operation {
            SequenceOperation::Create => {
                let options = sequence_options(&call, &args, false)?;
                (
                    format!(
                        "CREATE SEQUENCE{} {}{}{}",
                        if_not_exists_clause(args.if_not_exists),
                        name.sql(),
                        if options.is_empty() { "" } else { " " },
                        options
                    ),
                    &["CreateSeqStmt"],
                )
            }
            SequenceOperation::Alter => {
                let options = sequence_options(&call, &args, true)?;
                if options.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "operation".to_owned(),
                        detail: "alter needs at least one option to change".to_owned(),
                    }
                    .into());
                }
                (
                    format!("ALTER SEQUENCE {} {options}", name.sql()),
                    &["AlterSeqStmt"],
                )
            }
            SequenceOperation::Drop => (
                format!(
                    "DROP SEQUENCE{} {}{}",
                    if_exists_clause(args.if_exists),
                    name.sql(),
                    cascade_suffix(args.cascade)
                ),
                &["DropStmt"],
            ),
            SequenceOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER SEQUENCE {} RENAME TO {}",
                        name.sql(),
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
        };
        run_ddl(
            &call,
            "pg_sequence",
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
        route::<IndexArgs, crate::tools::write::StatementOutput, _>(
            &tool_specs::PG_INDEX,
            INDEX_DESCRIPTION,
            index,
        )?,
        route::<ViewArgs, crate::tools::write::StatementOutput, _>(
            &tool_specs::PG_VIEW,
            VIEW_DESCRIPTION,
            view,
        )?,
        route::<SequenceArgs, crate::tools::write::StatementOutput, _>(
            &tool_specs::PG_SEQUENCE,
            SEQUENCE_DESCRIPTION,
            sequence,
        )?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_elements_keep_their_order_suffix_and_wrap_expressions() {
        assert_eq!(index_element("email").unwrap(), "\"email\"");
        assert_eq!(
            index_element("created_at DESC").unwrap(),
            "\"created_at\" DESC"
        );
        assert_eq!(
            index_element("lower(email) nulls last").unwrap(),
            "(lower(email)) NULLS LAST"
        );
        assert!(index_element("email); DROP TABLE t; --").is_err());
    }
}
