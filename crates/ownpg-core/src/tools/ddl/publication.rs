use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Toggle, if_exists_clause, run_ddl, scoped_name};
use crate::error::{Error, Result};
use crate::render::{quote_ident, quote_literal, validate_ident};
use crate::shape::ResultSet;
use crate::tool_specs;
use crate::tools::{Call, Outcome, Route, route};

const PUBLICATION_DESCRIPTION: &str = "Create, alter, rename, or drop a logical replication publication, which belongs to the whole database, not to the scoped schema. create takes for_all_tables or a list of tables in the scoped schema, plus the optional publish list (insert, update, delete, truncate) and publish_via_partition_root. for_all_tables is the one argument on this tool that reaches outside the scoped schema: it publishes every table in every schema of the database, present and future, and needs a superuser connection to do it; a least-privilege connection, the kind this server recommends, gets PostgreSQL's own permission error back instead. add_tables appends to the table list and never needs confirm; set_tables replaces the whole table list, so any table left out stops replicating, and drop_tables removes named tables from the list, so both need confirm: true or the confirmation prompt, the same as drop. The table list a create or add_tables call names is fixed at that moment; it does not track tables created later, and this tool cannot express PostgreSQL's per-schema or row-filtered publication forms. Publications only ship changes to a subscriber that connects on its own; this server never creates a subscription, since a subscription would make the database host open an outbound connection to an address this server does not control. drop is destructive and needs confirm: true or the confirmation prompt.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PublicationOperation {
    Create,
    AddTables,
    SetTables,
    DropTables,
    Rename,
    SetOwner,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicationArgs {
    pub operation: PublicationOperation,
    #[schemars(
        description = "Publication name. A publication belongs to the whole database, not to the scoped schema."
    )]
    pub name: String,
    #[serde(default)]
    #[schemars(
        description = "create: publish every table in every schema of the database, present and future. The one argument on this tool that reaches outside the scoped schema; needs a superuser connection."
    )]
    pub for_all_tables: bool,
    #[serde(default)]
    #[schemars(
        description = "Tables in the scoped schema. Required for create unless for_all_tables is true, and for add_tables, set_tables, and drop_tables."
    )]
    pub tables: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "create only: comma list of insert, update, delete, truncate. Empty keeps the PostgreSQL default (every operation)."
    )]
    pub publish: String,
    #[serde(default)]
    #[schemars(
        description = "create only: ship a partitioned table's changes under its root name."
    )]
    pub publish_via_partition_root: Toggle,
    #[serde(default)]
    #[schemars(description = "set_owner: the new owner role.")]
    pub owner: String,
    #[serde(default)]
    #[schemars(description = "rename: the new name.")]
    pub new_name: String,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

fn publish_value(text: &str) -> Result<String> {
    const ALLOWED: [&str; 4] = ["insert", "update", "delete", "truncate"];
    let mut seen = Vec::new();
    for token in text.split(',') {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() {
            continue;
        }
        if !ALLOWED.contains(&token.as_str()) {
            return Err(Error::ArgumentInvalid {
                argument: "publish".to_owned(),
                detail: format!("`{token}` is not insert, update, delete, or truncate"),
            });
        }
        seen.push(token);
    }
    if seen.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "publish".to_owned(),
            detail: "publish needs at least one of insert, update, delete, truncate".to_owned(),
        });
    }
    Ok(seen.join(", "))
}

fn table_list(call: &Call, tables: &[String]) -> Result<String> {
    if tables.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "tables".to_owned(),
            detail: "at least one table is required".to_owned(),
        });
    }
    let mut rendered = Vec::with_capacity(tables.len());
    for table in tables {
        rendered.push(scoped_name(call, "tables", table)?.sql());
    }
    Ok(rendered.join(", "))
}

pub fn publication(call: Call, args: PublicationArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        validate_ident("name", &args.name)?;
        let name = quote_ident(&args.name);
        let (sql, kinds): (String, &[&str]) = match args.operation {
            PublicationOperation::Create => {
                if args.for_all_tables && !args.tables.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "tables".to_owned(),
                        detail: "for_all_tables and tables cannot both be set".to_owned(),
                    }
                    .into());
                }
                let target = if args.for_all_tables {
                    " FOR ALL TABLES".to_owned()
                } else if args.tables.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "tables".to_owned(),
                        detail: "create needs for_all_tables or at least one table".to_owned(),
                    }
                    .into());
                } else {
                    format!(" FOR TABLE {}", table_list(&call, &args.tables)?)
                };
                let mut with_options = Vec::new();
                if !args.publish.trim().is_empty() {
                    with_options.push(format!(
                        "publish = {}",
                        quote_literal(&publish_value(&args.publish)?)
                    ));
                }
                if let Some(root) = args.publish_via_partition_root.as_bool() {
                    with_options.push(format!("publish_via_partition_root = {root}"));
                }
                let with_clause = if with_options.is_empty() {
                    String::new()
                } else {
                    format!(" WITH ({})", with_options.join(", "))
                };
                (
                    format!("CREATE PUBLICATION {name}{target}{with_clause}"),
                    &["CreatePublicationStmt"],
                )
            }
            PublicationOperation::AddTables => (
                format!(
                    "ALTER PUBLICATION {name} ADD TABLE {}",
                    table_list(&call, &args.tables)?
                ),
                &["AlterPublicationStmt"],
            ),
            PublicationOperation::SetTables => (
                format!(
                    "ALTER PUBLICATION {name} SET TABLE {}",
                    table_list(&call, &args.tables)?
                ),
                &["AlterPublicationStmt"],
            ),
            PublicationOperation::DropTables => (
                format!(
                    "ALTER PUBLICATION {name} DROP TABLE {}",
                    table_list(&call, &args.tables)?
                ),
                &["AlterPublicationStmt"],
            ),
            PublicationOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER PUBLICATION {name} RENAME TO {}",
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            PublicationOperation::SetOwner => {
                validate_ident("owner", &args.owner)?;
                (
                    format!(
                        "ALTER PUBLICATION {name} OWNER TO {}",
                        quote_ident(&args.owner)
                    ),
                    &["AlterOwnerStmt"],
                )
            }
            PublicationOperation::Drop => (
                format!(
                    "DROP PUBLICATION{} {name}",
                    if_exists_clause(args.if_exists)
                ),
                &["DropStmt"],
            ),
        };
        run_ddl(
            &call,
            "pg_publication",
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
    Ok(vec![route::<PublicationArgs, ResultSet, _>(
        &tool_specs::PG_PUBLICATION,
        PUBLICATION_DESCRIPTION,
        publication,
    )?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_value_accepts_a_known_list_and_refuses_junk() {
        assert_eq!(publish_value("insert, update").unwrap(), "insert, update");
        assert_eq!(publish_value("DELETE").unwrap(), "delete");
        assert!(publish_value("insert, drop").is_err());
        assert!(publish_value("").is_err());
        assert!(publish_value("  ,  ").is_err());
    }
}
