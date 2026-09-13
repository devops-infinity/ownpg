use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    GeneratedKind, Identity, Missing, ReferentialAction, Toggle, cascade_suffix, if_exists_clause,
    if_not_exists_clause, run_ddl, scoped_name,
};
use crate::error::{Error, Result};
use crate::render::{expression, ident_list, quote_ident, type_name, validate_ident};
use crate::shape::ResultSet;
use crate::tool_specs;
use crate::tools::{Call, Outcome, Route, route};

const TABLE_DESCRIPTION: &str = "Create, alter, rename, truncate, or drop a table in the scoped schema, and attach or detach partitions. create takes columns with types, defaults, identity, generated expressions, and inline constraints, plus a composite primary key, partitioning, LIKE, and inheritance. drop, truncate, and detach_partition are destructive and need confirm: true or the confirmation prompt. dry_run returns the rendered SQL.";

const COLUMN_DESCRIPTION: &str = "Change one column of a table in the scoped schema: add, drop, rename, alter_type (with an optional USING expression), set_default, drop_default, set_not_null, drop_not_null, add_identity, drop_identity, set_statistics, set_storage, or set_compression. drop and a type change with USING are destructive and need confirm: true or the confirmation prompt.";

const CONSTRAINT_DESCRIPTION: &str = "Manage constraints on a table in the scoped schema: add a primary key, unique, check, foreign key, exclusion, or named not-null constraint (with NOT VALID, DEFERRABLE, NULLS NOT DISTINCT on 15 and later, WITHOUT OVERLAPS and NOT ENFORCED on 18 and later), then validate, rename, alter, or drop it. drop is destructive and needs confirm: true or the confirmation prompt.";

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnSpec {
    pub name: String,
    #[serde(default)]
    pub data_type: String,
    #[serde(default)]
    pub not_null: bool,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub primary_key: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub check: String,
    #[serde(default)]
    pub identity: Identity,
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub generated_kind: GeneratedKind,
    #[serde(default)]
    pub references: String,
    #[serde(default)]
    pub references_column: String,
    #[serde(default)]
    pub on_delete: ReferentialAction,
    #[serde(default)]
    pub on_update: ReferentialAction,
    #[serde(default)]
    pub collation: String,
}

pub fn column_definition(call: &Call, column: &ColumnSpec, kind: &str) -> Result<String> {
    validate_ident(kind, &column.name)?;
    let mut parts = vec![
        quote_ident(&column.name),
        type_name(kind, &column.data_type)?,
    ];
    if !column.collation.trim().is_empty() {
        validate_ident("collation", column.collation.trim())?;
        parts.push(format!("COLLATE {}", quote_ident(column.collation.trim())));
    }
    if !column.default.trim().is_empty() {
        parts.push(format!(
            "DEFAULT {}",
            expression("default", &column.default)?
        ));
    }
    if !column.generated.trim().is_empty() {
        let mode = match column.generated_kind {
            GeneratedKind::Virtual => {
                if !call
                    .engine()
                    .features()
                    .supports_virtual_generated_columns()
                {
                    return Err(Error::ArgumentInvalid {
                        argument: "generated_kind".to_owned(),
                        detail: "virtual generated columns need PostgreSQL 18 or later".to_owned(),
                    });
                }
                " VIRTUAL"
            }
            GeneratedKind::Stored => " STORED",
        };
        parts.push(format!(
            "GENERATED ALWAYS AS ({}){mode}",
            expression("generated", &column.generated)?
        ));
    }
    if column.identity != Identity::None {
        parts.push(column.identity.sql().to_owned());
    }
    if column.not_null {
        parts.push("NOT NULL".to_owned());
    }
    if column.primary_key {
        parts.push("PRIMARY KEY".to_owned());
    }
    if column.unique {
        parts.push("UNIQUE".to_owned());
    }
    if !column.check.trim().is_empty() {
        parts.push(format!("CHECK ({})", expression("check", &column.check)?));
    }
    if !column.references.trim().is_empty() {
        let target = scoped_name(call, "references", &column.references)?;
        let mut clause = format!("REFERENCES {}", target.sql());
        if !column.references_column.trim().is_empty() {
            validate_ident("references_column", column.references_column.trim())?;
            clause.push_str(&format!(
                " ({})",
                quote_ident(column.references_column.trim())
            ));
        }
        if column.on_delete != ReferentialAction::Unset {
            clause.push_str(&format!(" ON DELETE {}", column.on_delete.sql()));
        }
        if column.on_update != ReferentialAction::Unset {
            clause.push_str(&format!(" ON UPDATE {}", column.on_update.sql()));
        }
        parts.push(clause);
    }
    Ok(parts.join(" "))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TableOperation {
    Create,
    Drop,
    Rename,
    Truncate,
    SetLogged,
    SetUnlogged,
    SetOwner,
    AttachPartition,
    DetachPartition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PartitionMethod {
    #[default]
    None,
    Range,
    List,
    Hash,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableArgs {
    pub operation: TableOperation,
    #[schemars(description = "Table name, optionally schema-qualified.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "create: the columns.")]
    pub columns: Vec<ColumnSpec>,
    #[serde(default)]
    #[schemars(description = "create: columns of a composite primary key.")]
    pub primary_key: Vec<String>,
    #[serde(default)]
    pub if_not_exists: bool,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    pub temporary: bool,
    #[serde(default)]
    pub unlogged: bool,
    #[serde(default)]
    #[schemars(description = "create: partition the table by range, list, or hash.")]
    pub partition_by: PartitionMethod,
    #[serde(default)]
    #[schemars(description = "create: partition key columns or expressions.")]
    pub partition_key: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "create: copy the column layout of this table (LIKE ... INCLUDING ALL)."
    )]
    pub like: String,
    #[serde(default)]
    #[schemars(description = "create: parent tables to inherit from.")]
    pub inherits: Vec<String>,
    #[serde(default)]
    #[schemars(description = "rename: the new table name.")]
    pub new_name: String,
    #[serde(default)]
    #[schemars(description = "set_owner: the new owner role.")]
    pub owner: String,
    #[serde(default)]
    #[schemars(description = "truncate: reset identity and sequence values.")]
    pub restart_identity: bool,
    #[serde(default)]
    #[schemars(description = "attach_partition and detach_partition: the partition table.")]
    pub partition: String,
    #[serde(default)]
    #[schemars(
        description = "attach_partition: the bound, such as FOR VALUES FROM (1) TO (10), FOR VALUES IN ('a'), or DEFAULT."
    )]
    pub bound: String,
    #[serde(default)]
    #[schemars(description = "detach_partition: detach concurrently (PostgreSQL 14 and later).")]
    pub concurrently: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

pub fn table(call: Call, args: TableArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let name = scoped_name(&call, "name", &args.name)?;
        let (sql, kinds): (String, &[&str]) = match args.operation {
            TableOperation::Create => {
                let mut missing = Missing::new();
                missing.need(
                    "columns",
                    !args.columns.is_empty() || !args.like.trim().is_empty(),
                );
                missing.finish("create")?;
                let mut items = Vec::new();
                if !args.like.trim().is_empty() {
                    let source = scoped_name(&call, "like", &args.like)?;
                    items.push(format!("LIKE {} INCLUDING ALL", source.sql()));
                }
                for column in &args.columns {
                    items.push(column_definition(&call, column, "columns")?);
                }
                if !args.primary_key.is_empty() {
                    items.push(format!(
                        "PRIMARY KEY ({})",
                        ident_list("primary_key", &args.primary_key)?
                    ));
                }
                let mut sql = format!(
                    "CREATE{}{} TABLE{} {} ({})",
                    if args.temporary { " TEMPORARY" } else { "" },
                    if args.unlogged { " UNLOGGED" } else { "" },
                    if_not_exists_clause(args.if_not_exists),
                    name.sql(),
                    items.join(", ")
                );
                if !args.inherits.is_empty() {
                    let parents: Result<Vec<String>> = args
                        .inherits
                        .iter()
                        .map(|parent| scoped_name(&call, "inherits", parent).map(|n| n.sql()))
                        .collect();
                    sql.push_str(&format!(" INHERITS ({})", parents?.join(", ")));
                }
                if args.partition_by != PartitionMethod::None {
                    let keys: Result<Vec<String>> = args
                        .partition_key
                        .iter()
                        .map(|key| expression("partition_key", key))
                        .collect();
                    let keys = keys?;
                    if keys.is_empty() {
                        return Err(Error::ArgumentInvalid {
                            argument: "partition_key".to_owned(),
                            detail: "partition_by needs at least one key".to_owned(),
                        }
                        .into());
                    }
                    let method = match args.partition_by {
                        PartitionMethod::None | PartitionMethod::Range => "RANGE",
                        PartitionMethod::List => "LIST",
                        PartitionMethod::Hash => "HASH",
                    };
                    sql.push_str(&format!(" PARTITION BY {method} ({})", keys.join(", ")));
                }
                (sql, &["CreateStmt"])
            }
            TableOperation::Drop => (
                format!(
                    "DROP TABLE{} {}{}",
                    if_exists_clause(args.if_exists),
                    name.sql(),
                    cascade_suffix(args.cascade)
                ),
                &["DropStmt"],
            ),
            TableOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER TABLE {} RENAME TO {}",
                        name.sql(),
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            TableOperation::Truncate => (
                format!(
                    "TRUNCATE TABLE {}{}{}",
                    name.sql(),
                    if args.restart_identity {
                        " RESTART IDENTITY"
                    } else {
                        ""
                    },
                    cascade_suffix(args.cascade)
                ),
                &["TruncateStmt"],
            ),
            TableOperation::SetLogged => (
                format!("ALTER TABLE {} SET LOGGED", name.sql()),
                &["AlterTableStmt"],
            ),
            TableOperation::SetUnlogged => (
                format!("ALTER TABLE {} SET UNLOGGED", name.sql()),
                &["AlterTableStmt"],
            ),
            TableOperation::SetOwner => {
                validate_ident("owner", &args.owner)?;
                (
                    format!(
                        "ALTER TABLE {} OWNER TO {}",
                        name.sql(),
                        quote_ident(&args.owner)
                    ),
                    &["AlterTableStmt"],
                )
            }
            TableOperation::AttachPartition => {
                let mut missing = Missing::new();
                missing
                    .need("partition", !args.partition.trim().is_empty())
                    .need("bound", !args.bound.trim().is_empty());
                missing.finish("attach_partition")?;
                let partition = scoped_name(&call, "partition", &args.partition)?;
                (
                    format!(
                        "ALTER TABLE {} ATTACH PARTITION {} {}",
                        name.sql(),
                        partition.sql(),
                        args.bound.trim()
                    ),
                    &["AlterTableStmt"],
                )
            }
            TableOperation::DetachPartition => {
                let mut missing = Missing::new();
                missing.need("partition", !args.partition.trim().is_empty());
                missing.finish("detach_partition")?;
                let partition = scoped_name(&call, "partition", &args.partition)?;
                (
                    format!(
                        "ALTER TABLE {} DETACH PARTITION {}{}",
                        name.sql(),
                        partition.sql(),
                        if args.concurrently {
                            " CONCURRENTLY"
                        } else {
                            ""
                        }
                    ),
                    &["AlterTableStmt"],
                )
            }
        };
        run_ddl(
            &call,
            "pg_table",
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
pub enum ColumnOperation {
    Add,
    Drop,
    Rename,
    AlterType,
    SetDefault,
    DropDefault,
    SetNotNull,
    DropNotNull,
    AddIdentity,
    DropIdentity,
    SetStatistics,
    SetStorage,
    SetCompression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Storage {
    #[default]
    Unset,
    Plain,
    External,
    Extended,
    Main,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnArgs {
    pub operation: ColumnOperation,
    #[schemars(description = "Table name, optionally schema-qualified.")]
    pub table: String,
    #[schemars(description = "Column name.")]
    pub column: String,
    #[serde(default)]
    #[schemars(
        description = "add: the full column definition; the name field is ignored in favor of column."
    )]
    pub definition: ColumnSpec,
    #[serde(default)]
    #[schemars(description = "add and alter_type: the data type.")]
    pub data_type: String,
    #[serde(default)]
    #[schemars(description = "alter_type: expression that converts the old value.")]
    pub using: String,
    #[serde(default)]
    #[schemars(description = "set_default: the default expression.")]
    pub default: String,
    #[serde(default)]
    #[schemars(description = "rename: the new column name.")]
    pub new_name: String,
    #[serde(default)]
    #[schemars(description = "add_identity: always or by_default.")]
    pub identity: Identity,
    #[serde(default)]
    #[schemars(description = "set_statistics: the target, -1 for the default.")]
    pub statistics: i32,
    #[serde(default)]
    pub storage: Storage,
    #[serde(default)]
    #[schemars(description = "set_compression: pglz, lz4, or default.")]
    pub compression: String,
    #[serde(default)]
    pub if_not_exists: bool,
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

pub fn column(call: Call, args: ColumnArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let table = scoped_name(&call, "table", &args.table)?;
        validate_ident("column", &args.column)?;
        let column = quote_ident(&args.column);
        let head = format!("ALTER TABLE {} ", table.sql());
        let (sql, kinds): (String, &[&str]) = match args.operation {
            ColumnOperation::Add => {
                let mut spec = args.definition.clone();
                spec.name.clone_from(&args.column);
                if spec.data_type.trim().is_empty() {
                    spec.data_type.clone_from(&args.data_type);
                }
                let mut missing = Missing::new();
                missing.need("data_type", !spec.data_type.trim().is_empty());
                missing.finish("add")?;
                (
                    format!(
                        "{head}ADD COLUMN{} {}",
                        if_not_exists_clause(args.if_not_exists),
                        column_definition(&call, &spec, "definition")?
                    ),
                    &["AlterTableStmt"],
                )
            }
            ColumnOperation::Drop => (
                format!(
                    "{head}DROP COLUMN{} {column}{}",
                    if_exists_clause(args.if_exists),
                    cascade_suffix(args.cascade)
                ),
                &["AlterTableStmt"],
            ),
            ColumnOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "{head}RENAME COLUMN {column} TO {}",
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            ColumnOperation::AlterType => {
                let mut missing = Missing::new();
                missing.need("data_type", !args.data_type.trim().is_empty());
                missing.finish("alter_type")?;
                let mut sql = format!(
                    "{head}ALTER COLUMN {column} TYPE {}",
                    type_name("data_type", &args.data_type)?
                );
                if !args.using.trim().is_empty() {
                    sql.push_str(&format!(" USING {}", expression("using", &args.using)?));
                }
                (sql, &["AlterTableStmt"])
            }
            ColumnOperation::SetDefault => {
                let mut missing = Missing::new();
                missing.need("default", !args.default.trim().is_empty());
                missing.finish("set_default")?;
                (
                    format!(
                        "{head}ALTER COLUMN {column} SET DEFAULT {}",
                        expression("default", &args.default)?
                    ),
                    &["AlterTableStmt"],
                )
            }
            ColumnOperation::DropDefault => (
                format!("{head}ALTER COLUMN {column} DROP DEFAULT"),
                &["AlterTableStmt"],
            ),
            ColumnOperation::SetNotNull => (
                format!("{head}ALTER COLUMN {column} SET NOT NULL"),
                &["AlterTableStmt"],
            ),
            ColumnOperation::DropNotNull => (
                format!("{head}ALTER COLUMN {column} DROP NOT NULL"),
                &["AlterTableStmt"],
            ),
            ColumnOperation::AddIdentity => {
                if args.identity == Identity::None {
                    return Err(Error::ArgumentInvalid {
                        argument: "identity".to_owned(),
                        detail: "add_identity needs identity: always or by_default".to_owned(),
                    }
                    .into());
                }
                (
                    format!("{head}ALTER COLUMN {column} ADD {}", args.identity.sql()),
                    &["AlterTableStmt"],
                )
            }
            ColumnOperation::DropIdentity => (
                format!(
                    "{head}ALTER COLUMN {column} DROP IDENTITY{}",
                    if_exists_clause(args.if_exists)
                ),
                &["AlterTableStmt"],
            ),
            ColumnOperation::SetStatistics => (
                format!(
                    "{head}ALTER COLUMN {column} SET STATISTICS {}",
                    args.statistics.max(-1)
                ),
                &["AlterTableStmt"],
            ),
            ColumnOperation::SetStorage => {
                let word = match args.storage {
                    Storage::Unset => {
                        return Err(Error::ArgumentInvalid {
                            argument: "storage".to_owned(),
                            detail: "set_storage needs storage: plain, external, extended, main, or default".to_owned(),
                        }
                        .into());
                    }
                    Storage::Plain => "PLAIN",
                    Storage::External => "EXTERNAL",
                    Storage::Extended => "EXTENDED",
                    Storage::Main => "MAIN",
                    Storage::Default => "DEFAULT",
                };
                (
                    format!("{head}ALTER COLUMN {column} SET STORAGE {word}"),
                    &["AlterTableStmt"],
                )
            }
            ColumnOperation::SetCompression => {
                let method = args.compression.trim().to_ascii_lowercase();
                if !["pglz", "lz4", "default"].contains(&method.as_str()) {
                    return Err(Error::ArgumentInvalid {
                        argument: "compression".to_owned(),
                        detail: "set_compression needs pglz, lz4, or default".to_owned(),
                    }
                    .into());
                }
                (
                    format!("{head}ALTER COLUMN {column} SET COMPRESSION {method}"),
                    &["AlterTableStmt"],
                )
            }
        };
        run_ddl(
            &call,
            "pg_column",
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
pub enum ConstraintOperation {
    Add,
    Validate,
    Drop,
    Rename,
    Alter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    #[default]
    Unset,
    PrimaryKey,
    Unique,
    Check,
    ForeignKey,
    Exclude,
    NotNull,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConstraintArgs {
    pub operation: ConstraintOperation,
    #[schemars(description = "Table name, optionally schema-qualified.")]
    pub table: String,
    #[serde(default)]
    #[schemars(description = "Constraint name. Optional for add; PostgreSQL names it otherwise.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "add: the constraint kind.")]
    pub kind: ConstraintKind,
    #[serde(default)]
    #[schemars(
        description = "add: constrained columns (primary_key, unique, foreign_key, not_null)."
    )]
    pub columns: Vec<String>,
    #[serde(default)]
    #[schemars(description = "add check: the boolean expression.")]
    pub expression: String,
    #[serde(default)]
    #[schemars(description = "add foreign_key: the referenced table.")]
    pub references: String,
    #[serde(default)]
    #[schemars(description = "add foreign_key: the referenced columns.")]
    pub references_columns: Vec<String>,
    #[serde(default)]
    pub on_delete: ReferentialAction,
    #[serde(default)]
    pub on_update: ReferentialAction,
    #[serde(default)]
    #[schemars(description = "add foreign_key: MATCH FULL instead of MATCH SIMPLE.")]
    pub match_full: bool,
    #[serde(default)]
    #[schemars(description = "add exclude: the access method, such as gist.")]
    pub method: String,
    #[serde(default)]
    #[schemars(
        description = "add exclude: elements such as \"room WITH =\" and \"during WITH &&\"."
    )]
    pub elements: Vec<String>,
    #[serde(default)]
    #[schemars(description = "add exclude: a WHERE predicate.")]
    pub where_clause: String,
    #[serde(default)]
    pub deferrable: bool,
    #[serde(default)]
    pub initially_deferred: bool,
    #[serde(default)]
    #[schemars(
        description = "add check or foreign_key: skip the scan of existing rows; validate later."
    )]
    pub not_valid: bool,
    #[serde(default)]
    #[schemars(description = "add unique: NULLS NOT DISTINCT (PostgreSQL 15 and later).")]
    pub nulls_not_distinct: bool,
    #[serde(default)]
    #[schemars(
        description = "add primary_key or unique: the range or period column for WITHOUT OVERLAPS (PostgreSQL 18 and later)."
    )]
    pub without_overlaps: String,
    #[serde(default)]
    #[schemars(
        description = "add foreign_key: the local and referenced PERIOD column (PostgreSQL 18 and later)."
    )]
    pub period: String,
    #[serde(default)]
    #[schemars(
        description = "add or alter check and foreign_key: false renders NOT ENFORCED (PostgreSQL 18 and later)."
    )]
    pub enforced: Toggle,
    #[serde(default)]
    #[schemars(description = "rename: the new constraint name.")]
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

fn constraint_body(call: &Call, args: &ConstraintArgs) -> Result<String> {
    let features = call.engine().features();
    let kind = args.kind;
    let body = match kind {
        ConstraintKind::Unset => {
            return Err(Error::ArgumentInvalid {
                argument: "kind".to_owned(),
                detail:
                    "add needs kind: primary_key, unique, check, foreign_key, exclude, or not_null"
                        .to_owned(),
            });
        }
        ConstraintKind::PrimaryKey | ConstraintKind::Unique => {
            let mut missing = Missing::new();
            missing.need("columns", !args.columns.is_empty());
            missing.finish("add")?;
            let mut columns = ident_list("columns", &args.columns)?;
            if !args.without_overlaps.trim().is_empty() {
                if !features.supports_without_overlaps() {
                    return Err(Error::ArgumentInvalid {
                        argument: "without_overlaps".to_owned(),
                        detail: "WITHOUT OVERLAPS needs PostgreSQL 18 or later".to_owned(),
                    });
                }
                validate_ident("without_overlaps", args.without_overlaps.trim())?;
                columns.push_str(&format!(
                    ", {} WITHOUT OVERLAPS",
                    quote_ident(args.without_overlaps.trim())
                ));
            }
            let nulls = if kind == ConstraintKind::Unique && args.nulls_not_distinct {
                if !features.supports_nulls_not_distinct() {
                    return Err(Error::ArgumentInvalid {
                        argument: "nulls_not_distinct".to_owned(),
                        detail: "NULLS NOT DISTINCT needs PostgreSQL 15 or later".to_owned(),
                    });
                }
                " NULLS NOT DISTINCT"
            } else {
                ""
            };
            let word = if kind == ConstraintKind::PrimaryKey {
                "PRIMARY KEY"
            } else {
                "UNIQUE"
            };
            format!("{word}{nulls} ({columns})")
        }
        ConstraintKind::Check => {
            let mut missing = Missing::new();
            missing.need("expression", !args.expression.trim().is_empty());
            missing.finish("add")?;
            format!("CHECK ({})", expression("expression", &args.expression)?)
        }
        ConstraintKind::NotNull => {
            let mut missing = Missing::new();
            missing.need("columns", args.columns.len() == 1);
            missing.finish("add")?;
            format!("NOT NULL {}", ident_list("columns", &args.columns)?)
        }
        ConstraintKind::ForeignKey => {
            let mut missing = Missing::new();
            missing
                .need("columns", !args.columns.is_empty())
                .need("references", !args.references.trim().is_empty());
            missing.finish("add")?;
            let target = scoped_name(call, "references", &args.references)?;
            let mut local = ident_list("columns", &args.columns)?;
            let mut remote = if args.references_columns.is_empty() {
                String::new()
            } else {
                format!(
                    " ({})",
                    ident_list("references_columns", &args.references_columns)?
                )
            };
            if !args.period.trim().is_empty() {
                if !features.supports_without_overlaps() {
                    return Err(Error::ArgumentInvalid {
                        argument: "period".to_owned(),
                        detail: "PERIOD foreign keys need PostgreSQL 18 or later".to_owned(),
                    });
                }
                validate_ident("period", args.period.trim())?;
                let period = quote_ident(args.period.trim());
                local.push_str(&format!(", PERIOD {period}"));
                if remote.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "references_columns".to_owned(),
                        detail: "a PERIOD foreign key names the referenced columns".to_owned(),
                    });
                }
                remote.insert_str(remote.len() - 1, &format!(", PERIOD {period}"));
            }
            let mut body = format!("FOREIGN KEY ({local}) REFERENCES {}{remote}", target.sql());
            if args.match_full {
                body.push_str(" MATCH FULL");
            }
            if args.on_delete != ReferentialAction::Unset {
                body.push_str(&format!(" ON DELETE {}", args.on_delete.sql()));
            }
            if args.on_update != ReferentialAction::Unset {
                body.push_str(&format!(" ON UPDATE {}", args.on_update.sql()));
            }
            body
        }
        ConstraintKind::Exclude => {
            let mut missing = Missing::new();
            missing.need("elements", !args.elements.is_empty());
            missing.finish("add")?;
            let method = if args.method.trim().is_empty() {
                String::new()
            } else {
                validate_ident("method", args.method.trim())?;
                format!("USING {} ", args.method.trim())
            };
            let mut body = format!("EXCLUDE {method}({})", args.elements.join(", "));
            if !args.where_clause.trim().is_empty() {
                body.push_str(&format!(
                    " WHERE ({})",
                    expression("where_clause", &args.where_clause)?
                ));
            }
            body
        }
    };
    let mut body = body;
    if let Some(enforced) = args.enforced.as_bool() {
        if !features.supports_not_enforced_constraints() {
            return Err(Error::ArgumentInvalid {
                argument: "enforced".to_owned(),
                detail: "ENFORCED and NOT ENFORCED need PostgreSQL 18 or later".to_owned(),
            });
        }
        body.push_str(if enforced {
            " ENFORCED"
        } else {
            " NOT ENFORCED"
        });
    }
    if args.deferrable {
        body.push_str(" DEFERRABLE");
        if args.initially_deferred {
            body.push_str(" INITIALLY DEFERRED");
        }
    }
    if args.not_valid {
        body.push_str(" NOT VALID");
    }
    Ok(body)
}

pub fn constraint(call: Call, args: ConstraintArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let table = scoped_name(&call, "table", &args.table)?;
        let head = format!("ALTER TABLE {} ", table.sql());
        let named = |field: &str| -> Result<String> {
            validate_ident(field, args.name.trim())?;
            Ok(quote_ident(args.name.trim()))
        };
        let (sql, kinds): (String, &[&str]) = match args.operation {
            ConstraintOperation::Add => {
                let body = constraint_body(&call, &args)?;
                let label = if args.name.trim().is_empty() {
                    String::new()
                } else {
                    format!("CONSTRAINT {} ", named("name")?)
                };
                (format!("{head}ADD {label}{body}"), &["AlterTableStmt"])
            }
            ConstraintOperation::Validate => (
                format!("{head}VALIDATE CONSTRAINT {}", named("name")?),
                &["AlterTableStmt"],
            ),
            ConstraintOperation::Drop => (
                format!(
                    "{head}DROP CONSTRAINT{} {}{}",
                    if_exists_clause(args.if_exists),
                    named("name")?,
                    cascade_suffix(args.cascade)
                ),
                &["AlterTableStmt"],
            ),
            ConstraintOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "{head}RENAME CONSTRAINT {} TO {}",
                        named("name")?,
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            ConstraintOperation::Alter => {
                let mut clauses = Vec::new();
                if let Some(enforced) = args.enforced.as_bool() {
                    if !call.engine().features().supports_not_enforced_constraints() {
                        return Err(Error::ArgumentInvalid {
                            argument: "enforced".to_owned(),
                            detail: "ENFORCED and NOT ENFORCED need PostgreSQL 18 or later"
                                .to_owned(),
                        }
                        .into());
                    }
                    clauses.push(if enforced { "ENFORCED" } else { "NOT ENFORCED" });
                }
                if args.deferrable {
                    clauses.push("DEFERRABLE");
                    clauses.push(if args.initially_deferred {
                        "INITIALLY DEFERRED"
                    } else {
                        "INITIALLY IMMEDIATE"
                    });
                }
                if clauses.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "operation".to_owned(),
                        detail: "alter needs deferrable or enforced".to_owned(),
                    }
                    .into());
                }
                (
                    format!(
                        "{head}ALTER CONSTRAINT {} {}",
                        named("name")?,
                        clauses.join(" ")
                    ),
                    &["AlterTableStmt"],
                )
            }
        };
        run_ddl(
            &call,
            "pg_constraint",
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
        route::<TableArgs, ResultSet, _>(&tool_specs::PG_TABLE, TABLE_DESCRIPTION, table)?,
        route::<ColumnArgs, ResultSet, _>(&tool_specs::PG_COLUMN, COLUMN_DESCRIPTION, column)?,
        route::<ConstraintArgs, ResultSet, _>(
            &tool_specs::PG_CONSTRAINT,
            CONSTRAINT_DESCRIPTION,
            constraint,
        )?,
    ])
}
