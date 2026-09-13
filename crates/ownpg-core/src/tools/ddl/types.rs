use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    Missing, cascade_suffix, if_exists_clause, if_not_exists_clause, run_ddl, scoped_name,
};
use crate::error::{Error, Result};
use crate::render::{expression, quote_ident, quote_literal, type_name, validate_ident};
use crate::shape::ResultSet;
use crate::tool_specs;
use crate::tools::catalog;
use crate::tools::{AuditFacts, Call, Outcome, Route, ToolOutput, route, text_rows};

const TYPE_DESCRIPTION: &str = "Create, alter, rename, or drop a type in the scoped schema: enum (create_enum, add_value, rename_value), composite (create_composite, add_attribute, drop_attribute, alter_attribute), domain (create_domain, set_default, drop_default, set_not_null, drop_not_null, add_check, drop_constraint, validate_constraint), and range (create_range). An enum value added inside a transaction cannot be used until that transaction commits (SQLSTATE 55P04), so add the value in one call and use it in the next. drop is destructive and needs confirm: true or the confirmation prompt.";

const EXTENSION_DESCRIPTION: &str = "Create, update, or drop an extension, or list the extensions available on the server. create installs the extension into the scoped schema unless the extension is not relocatable. A short, fixed list of extensions that grant OS-level or arbitrary-network access is refused. drop is destructive and needs confirm: true or the confirmation prompt.";

const DENIED_EXTENSIONS: [&str; 8] = [
    "plpythonu",
    "plpython3u",
    "plperlu",
    "pltclu",
    "dblink",
    "postgres_fdw",
    "file_fdw",
    "adminpack",
];

fn refuse_dangerous_extension(name: &str) -> Result<()> {
    if DENIED_EXTENSIONS.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(Error::ArgumentInvalid {
            argument: "name".to_owned(),
            detail: format!("`{name}` grants OS-level or arbitrary-network access and is refused"),
        });
    }
    Ok(())
}

const COMMENT_DESCRIPTION: &str = "Set or remove the comment on an object in the scoped schema: a table, column, index, view, materialized view, sequence, function, procedure, type, domain, constraint, trigger, policy, or the schema itself, and on a role, an extension, or the database.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TypeOperation {
    CreateEnum,
    AddValue,
    RenameValue,
    CreateComposite,
    AddAttribute,
    DropAttribute,
    AlterAttribute,
    CreateDomain,
    SetDefault,
    DropDefault,
    SetNotNull,
    DropNotNull,
    AddCheck,
    DropConstraint,
    ValidateConstraint,
    CreateRange,
    Rename,
    SetOwner,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub collation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeArgs {
    pub operation: TypeOperation,
    #[schemars(description = "Type name, optionally schema-qualified.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "create_enum: the labels in order.")]
    pub labels: Vec<String>,
    #[serde(default)]
    #[schemars(description = "add_value and rename_value: the label.")]
    pub value: String,
    #[serde(default)]
    #[schemars(description = "rename_value: the new label.")]
    pub new_value: String,
    #[serde(default)]
    #[schemars(description = "add_value: place the label before this one.")]
    pub before: String,
    #[serde(default)]
    #[schemars(description = "add_value: place the label after this one.")]
    pub after: String,
    #[serde(default)]
    #[schemars(description = "create_composite: the attributes.")]
    pub attributes: Vec<Attribute>,
    #[serde(default)]
    #[schemars(description = "add_attribute, drop_attribute, alter_attribute: the attribute.")]
    pub attribute: Attribute,
    #[serde(default)]
    #[schemars(description = "create_domain and create_range: the base or subtype.")]
    pub base_type: String,
    #[serde(default)]
    #[schemars(description = "create_domain and set_default: the default expression.")]
    pub default: String,
    #[serde(default)]
    #[schemars(description = "create_domain: NOT NULL.")]
    pub not_null: bool,
    #[serde(default)]
    #[schemars(description = "create_domain and add_check: the check expression over VALUE.")]
    pub check: String,
    #[serde(default)]
    #[schemars(
        description = "add_check, drop_constraint, validate_constraint: the constraint name."
    )]
    pub constraint_name: String,
    #[serde(default)]
    pub collation: String,
    #[serde(default)]
    #[schemars(description = "create_range: the subtype operator class.")]
    pub subtype_opclass: String,
    #[serde(default)]
    #[schemars(description = "create_range: the canonical function.")]
    pub canonical: String,
    #[serde(default)]
    #[schemars(description = "create_range: the subtype difference function.")]
    pub subtype_diff: String,
    #[serde(default)]
    #[schemars(description = "create_range: the multirange type name (PostgreSQL 14 and later).")]
    pub multirange_type_name: String,
    #[serde(default)]
    #[schemars(description = "rename: the new name.")]
    pub new_name: String,
    #[serde(default)]
    #[schemars(description = "set_owner: the new owner.")]
    pub owner: String,
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

fn attribute_sql(attribute: &Attribute) -> Result<String> {
    validate_ident("attributes", &attribute.name)?;
    let mut out = format!(
        "{} {}",
        quote_ident(&attribute.name),
        type_name("attributes", &attribute.data_type)?
    );
    if !attribute.collation.trim().is_empty() {
        validate_ident("collation", attribute.collation.trim())?;
        out.push_str(&format!(
            " COLLATE {}",
            quote_ident(attribute.collation.trim())
        ));
    }
    Ok(out)
}

pub fn type_tool(call: Call, args: TypeArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let name = scoped_name(&call, "name", &args.name)?;
        let head = format!("ALTER TYPE {} ", name.sql());
        let domain = format!("ALTER DOMAIN {} ", name.sql());
        let attribute = |field: &str| -> Result<Attribute> {
            if args.attribute.name.trim().is_empty() {
                return Err(Error::ArgumentInvalid {
                    argument: "attribute".to_owned(),
                    detail: format!("{field} needs attribute with a name"),
                });
            }
            Ok(args.attribute.clone())
        };
        let constraint = |field: &str| -> Result<String> {
            let mut missing = Missing::new();
            missing.need("constraint_name", !args.constraint_name.trim().is_empty());
            missing.finish(field)?;
            validate_ident("constraint_name", args.constraint_name.trim())?;
            Ok(quote_ident(args.constraint_name.trim()))
        };
        let (sql, kinds): (String, &[&str]) = match args.operation {
            TypeOperation::CreateEnum => {
                let labels: Vec<String> = args.labels.iter().map(|l| quote_literal(l)).collect();
                (
                    format!("CREATE TYPE {} AS ENUM ({})", name.sql(), labels.join(", ")),
                    &["CreateEnumStmt"],
                )
            }
            TypeOperation::AddValue => {
                let mut missing = Missing::new();
                missing.need("value", !args.value.is_empty());
                missing.finish("add_value")?;
                let mut sql = format!(
                    "{head}ADD VALUE{} {}",
                    if_not_exists_clause(args.if_not_exists),
                    quote_literal(&args.value)
                );
                if !args.before.is_empty() {
                    sql.push_str(&format!(" BEFORE {}", quote_literal(&args.before)));
                } else if !args.after.is_empty() {
                    sql.push_str(&format!(" AFTER {}", quote_literal(&args.after)));
                }
                (sql, &["AlterEnumStmt"])
            }
            TypeOperation::RenameValue => {
                let mut missing = Missing::new();
                missing
                    .need("value", !args.value.is_empty())
                    .need("new_value", !args.new_value.is_empty());
                missing.finish("rename_value")?;
                (
                    format!(
                        "{head}RENAME VALUE {} TO {}",
                        quote_literal(&args.value),
                        quote_literal(&args.new_value)
                    ),
                    &["AlterEnumStmt"],
                )
            }
            TypeOperation::CreateComposite => {
                let mut missing = Missing::new();
                missing.need("attributes", !args.attributes.is_empty());
                missing.finish("create_composite")?;
                let attributes: Result<Vec<String>> =
                    args.attributes.iter().map(attribute_sql).collect();
                (
                    format!("CREATE TYPE {} AS ({})", name.sql(), attributes?.join(", ")),
                    &["CompositeTypeStmt"],
                )
            }
            TypeOperation::AddAttribute => (
                format!(
                    "{head}ADD ATTRIBUTE {}",
                    attribute_sql(&attribute("add_attribute")?)?
                ),
                &["AlterTableStmt"],
            ),
            TypeOperation::DropAttribute => {
                let attribute = attribute("drop_attribute")?;
                validate_ident("attribute", &attribute.name)?;
                (
                    format!(
                        "{head}DROP ATTRIBUTE{} {}{}",
                        if_exists_clause(args.if_exists),
                        quote_ident(&attribute.name),
                        cascade_suffix(args.cascade)
                    ),
                    &["AlterTableStmt"],
                )
            }
            TypeOperation::AlterAttribute => {
                let attribute = attribute("alter_attribute")?;
                validate_ident("attribute", &attribute.name)?;
                (
                    format!(
                        "{head}ALTER ATTRIBUTE {} SET DATA TYPE {}{}",
                        quote_ident(&attribute.name),
                        type_name("attribute", &attribute.data_type)?,
                        cascade_suffix(args.cascade)
                    ),
                    &["AlterTableStmt"],
                )
            }
            TypeOperation::CreateDomain => {
                let mut missing = Missing::new();
                missing.need("base_type", !args.base_type.trim().is_empty());
                missing.finish("create_domain")?;
                let mut sql = format!(
                    "CREATE DOMAIN {} AS {}",
                    name.sql(),
                    type_name("base_type", &args.base_type)?
                );
                if !args.collation.trim().is_empty() {
                    validate_ident("collation", args.collation.trim())?;
                    sql.push_str(&format!(" COLLATE {}", quote_ident(args.collation.trim())));
                }
                if !args.default.trim().is_empty() {
                    sql.push_str(&format!(
                        " DEFAULT {}",
                        expression("default", &args.default)?
                    ));
                }
                if args.not_null {
                    sql.push_str(" NOT NULL");
                }
                if !args.check.trim().is_empty() {
                    if !args.constraint_name.trim().is_empty() {
                        validate_ident("constraint_name", args.constraint_name.trim())?;
                        sql.push_str(&format!(
                            " CONSTRAINT {}",
                            quote_ident(args.constraint_name.trim())
                        ));
                    }
                    sql.push_str(&format!(" CHECK ({})", expression("check", &args.check)?));
                }
                (sql, &["CreateDomainStmt"])
            }
            TypeOperation::SetDefault => {
                let mut missing = Missing::new();
                missing.need("default", !args.default.trim().is_empty());
                missing.finish("set_default")?;
                (
                    format!(
                        "{domain}SET DEFAULT {}",
                        expression("default", &args.default)?
                    ),
                    &["AlterDomainStmt"],
                )
            }
            TypeOperation::DropDefault => (format!("{domain}DROP DEFAULT"), &["AlterDomainStmt"]),
            TypeOperation::SetNotNull => (format!("{domain}SET NOT NULL"), &["AlterDomainStmt"]),
            TypeOperation::DropNotNull => (format!("{domain}DROP NOT NULL"), &["AlterDomainStmt"]),
            TypeOperation::AddCheck => {
                let mut missing = Missing::new();
                missing.need("check", !args.check.trim().is_empty());
                missing.finish("add_check")?;
                let label = if args.constraint_name.trim().is_empty() {
                    String::new()
                } else {
                    format!("CONSTRAINT {} ", constraint("add_check")?)
                };
                (
                    format!(
                        "{domain}ADD {label}CHECK ({})",
                        expression("check", &args.check)?
                    ),
                    &["AlterDomainStmt"],
                )
            }
            TypeOperation::DropConstraint => (
                format!(
                    "{domain}DROP CONSTRAINT{} {}{}",
                    if_exists_clause(args.if_exists),
                    constraint("drop_constraint")?,
                    cascade_suffix(args.cascade)
                ),
                &["AlterDomainStmt"],
            ),
            TypeOperation::ValidateConstraint => (
                format!(
                    "{domain}VALIDATE CONSTRAINT {}",
                    constraint("validate_constraint")?
                ),
                &["AlterDomainStmt"],
            ),
            TypeOperation::CreateRange => {
                let mut missing = Missing::new();
                missing.need("base_type", !args.base_type.trim().is_empty());
                missing.finish("create_range")?;
                let mut options = vec![format!(
                    "SUBTYPE = {}",
                    type_name("base_type", &args.base_type)?
                )];
                if !args.subtype_opclass.trim().is_empty() {
                    validate_ident("subtype_opclass", args.subtype_opclass.trim())?;
                    options.push(format!(
                        "SUBTYPE_OPCLASS = {}",
                        quote_ident(args.subtype_opclass.trim())
                    ));
                }
                if !args.collation.trim().is_empty() {
                    validate_ident("collation", args.collation.trim())?;
                    options.push(format!(
                        "COLLATION = {}",
                        quote_ident(args.collation.trim())
                    ));
                }
                if !args.canonical.trim().is_empty() {
                    let function = scoped_name(&call, "canonical", &args.canonical)?;
                    options.push(format!("CANONICAL = {}", function.sql()));
                }
                if !args.subtype_diff.trim().is_empty() {
                    let function = scoped_name(&call, "subtype_diff", &args.subtype_diff)?;
                    options.push(format!("SUBTYPE_DIFF = {}", function.sql()));
                }
                if !args.multirange_type_name.trim().is_empty() {
                    let multirange =
                        scoped_name(&call, "multirange_type_name", &args.multirange_type_name)?;
                    options.push(format!("MULTIRANGE_TYPE_NAME = {}", multirange.sql()));
                }
                (
                    format!(
                        "CREATE TYPE {} AS RANGE ({})",
                        name.sql(),
                        options.join(", ")
                    ),
                    &["CreateRangeStmt"],
                )
            }
            TypeOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!("{head}RENAME TO {}", quote_ident(&args.new_name)),
                    &["RenameStmt"],
                )
            }
            TypeOperation::SetOwner => {
                validate_ident("owner", &args.owner)?;
                (
                    format!("{head}OWNER TO {}", quote_ident(&args.owner)),
                    &["AlterOwnerStmt"],
                )
            }
            TypeOperation::Drop => (
                format!(
                    "DROP TYPE{} {}{}",
                    if_exists_clause(args.if_exists),
                    name.sql(),
                    cascade_suffix(args.cascade)
                ),
                &["DropStmt"],
            ),
        };
        run_ddl(
            &call,
            "pg_type",
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
pub enum ExtensionOperation {
    Create,
    Update,
    Drop,
    ListAvailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionArgs {
    pub operation: ExtensionOperation,
    #[serde(default)]
    #[schemars(description = "Extension name (not for available).")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "create and update: the version.")]
    pub version: String,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    pub if_not_exists: bool,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct AvailableExtension {
    pub name: String,
    pub default_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct AvailableExtensions {
    pub rows: Vec<AvailableExtension>,
    pub row_count: usize,
    pub order: &'static str,
    pub notice: &'static str,
}

pub fn extension(call: Call, args: ExtensionArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if args.operation == ExtensionOperation::ListAvailable {
            let rows = call
                .engine()
                .catalog_rows(
                    "SELECT name::text, default_version::text, installed_version::text, comment::text \
                     FROM pg_catalog.pg_available_extensions ORDER BY name",
                    &[],
                )
                .await?;
            let mut listed = Vec::new();
            for row in &rows {
                listed.push(AvailableExtension {
                    name: catalog::read_column(row, 0)?,
                    default_version: catalog::read_column(row, 1)?,
                    installed_version: catalog::read_column(row, 2)?,
                    comment: catalog::read_column(row, 3)?,
                });
            }
            let text = text_rows(
                &[
                    ("name", "text"),
                    ("default_version", "text"),
                    ("installed_version", "text"),
                    ("comment", "text"),
                ],
                listed
                    .iter()
                    .map(|extension| {
                        vec![
                            Some(extension.name.clone()),
                            Some(extension.default_version.clone()),
                            extension.installed_version.clone(),
                            extension.comment.clone(),
                        ]
                    })
                    .collect(),
                None,
                None,
            );
            let available = AvailableExtensions {
                row_count: listed.len(),
                rows: listed,
                order: "name",
                notice: crate::shape::UNTRUSTED_NOTICE,
            };
            return Ok(ToolOutput::structured(&available, text)?
                .with_facts(AuditFacts {
                    operation: Some("available".to_owned()),
                    row_count: Some(available.row_count as u64),
                    ..AuditFacts::default()
                })
                .into());
        }
        validate_ident("name", &args.name)?;
        if args.operation == ExtensionOperation::Create {
            refuse_dangerous_extension(&args.name)?;
        }
        let name = quote_ident(&args.name);
        let version = if args.version.trim().is_empty() {
            String::new()
        } else {
            format!(" VERSION {}", quote_literal(args.version.trim()))
        };
        let (sql, kinds): (String, &[&str]) = match args.operation {
            ExtensionOperation::Create => (
                format!(
                    "CREATE EXTENSION{} {name} SCHEMA {}{version}{}",
                    if_not_exists_clause(args.if_not_exists),
                    quote_ident(&call.settings().schema.value),
                    cascade_suffix(args.cascade)
                ),
                &["CreateExtensionStmt"],
            ),
            ExtensionOperation::Update => (
                format!(
                    "ALTER EXTENSION {name} UPDATE{}",
                    if version.is_empty() {
                        String::new()
                    } else {
                        format!(" TO {}", quote_literal(args.version.trim()))
                    }
                ),
                &["AlterExtensionStmt"],
            ),
            ExtensionOperation::Drop => (
                format!(
                    "DROP EXTENSION{} {name}{}",
                    if_exists_clause(args.if_exists),
                    cascade_suffix(args.cascade)
                ),
                &["DropStmt"],
            ),
            ExtensionOperation::ListAvailable => (String::new(), &[]),
        };
        run_ddl(
            &call,
            "pg_extension",
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
pub enum CommentTarget {
    Table,
    Column,
    Index,
    View,
    MaterializedView,
    Sequence,
    Function,
    Procedure,
    Type,
    Domain,
    Constraint,
    Trigger,
    Policy,
    Schema,
    Role,
    Extension,
    Database,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentArgs {
    pub target: CommentTarget,
    #[serde(default)]
    #[schemars(
        description = "Object name; for column, constraint, trigger, and policy the owning table; for schema and database the scoped one is used when empty."
    )]
    pub name: String,
    #[serde(default)]
    #[schemars(
        description = "column: the column; constraint, trigger, policy: the constraint, trigger, or policy name."
    )]
    pub member: String,
    #[serde(default)]
    #[schemars(
        description = "function and procedure: the argument types that identify the overload."
    )]
    pub argument_types: Vec<String>,
    #[serde(default)]
    #[schemars(description = "The comment text. Empty removes the comment.")]
    pub comment: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub transaction: String,
}

pub fn comment(call: Call, args: CommentArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        let member = |field: &str| -> Result<String> {
            let mut missing = Missing::new();
            missing.need("member", !args.member.trim().is_empty());
            missing.finish(field)?;
            validate_ident("member", args.member.trim())?;
            Ok(quote_ident(args.member.trim()))
        };
        let object = match args.target {
            CommentTarget::Table => {
                format!("TABLE {}", scoped_name(&call, "name", &args.name)?.sql())
            }
            CommentTarget::Column => format!(
                "COLUMN {}.{}",
                scoped_name(&call, "name", &args.name)?.sql(),
                member("column")?
            ),
            CommentTarget::Index => {
                format!("INDEX {}", scoped_name(&call, "name", &args.name)?.sql())
            }
            CommentTarget::View => {
                format!("VIEW {}", scoped_name(&call, "name", &args.name)?.sql())
            }
            CommentTarget::MaterializedView => format!(
                "MATERIALIZED VIEW {}",
                scoped_name(&call, "name", &args.name)?.sql()
            ),
            CommentTarget::Sequence => {
                format!("SEQUENCE {}", scoped_name(&call, "name", &args.name)?.sql())
            }
            CommentTarget::Function | CommentTarget::Procedure => {
                let types: Result<Vec<String>> = args
                    .argument_types
                    .iter()
                    .map(|t| type_name("argument_types", t))
                    .collect();
                format!(
                    "{} {}({})",
                    if args.target == CommentTarget::Function {
                        "FUNCTION"
                    } else {
                        "PROCEDURE"
                    },
                    scoped_name(&call, "name", &args.name)?.sql(),
                    types?.join(", ")
                )
            }
            CommentTarget::Type => {
                format!("TYPE {}", scoped_name(&call, "name", &args.name)?.sql())
            }
            CommentTarget::Domain => {
                format!("DOMAIN {}", scoped_name(&call, "name", &args.name)?.sql())
            }
            CommentTarget::Constraint => format!(
                "CONSTRAINT {} ON {}",
                member("constraint")?,
                scoped_name(&call, "name", &args.name)?.sql()
            ),
            CommentTarget::Trigger => format!(
                "TRIGGER {} ON {}",
                member("trigger")?,
                scoped_name(&call, "name", &args.name)?.sql()
            ),
            CommentTarget::Policy => format!(
                "POLICY {} ON {}",
                member("policy")?,
                scoped_name(&call, "name", &args.name)?.sql()
            ),
            CommentTarget::Schema => {
                let schema = if args.name.trim().is_empty() {
                    scoped.clone()
                } else {
                    args.name.trim().to_owned()
                };
                if schema != scoped {
                    return Err(Error::StatementRefused {
                        rule: format!("schema `{schema}` is outside the scoped schema `{scoped}`"),
                        mode: call.settings().mode.value.to_string(),
                    }
                    .into());
                }
                format!("SCHEMA {}", quote_ident(&schema))
            }
            CommentTarget::Role => {
                validate_ident("name", args.name.trim())?;
                format!("ROLE {}", quote_ident(args.name.trim()))
            }
            CommentTarget::Extension => {
                validate_ident("name", args.name.trim())?;
                format!("EXTENSION {}", quote_ident(args.name.trim()))
            }
            CommentTarget::Database => {
                let database = call.settings().database.value.clone();
                if !args.name.trim().is_empty() && args.name.trim() != database {
                    return Err(Error::StatementRefused {
                        rule: format!("database `{}` is not the served database", args.name.trim()),
                        mode: call.settings().mode.value.to_string(),
                    }
                    .into());
                }
                format!("DATABASE {}", quote_ident(&database))
            }
        };
        let value = if args.comment.is_empty() {
            "NULL".to_owned()
        } else {
            quote_literal(&args.comment)
        };
        let sql = format!("COMMENT ON {object} IS {value}");
        run_ddl(
            &call,
            "pg_comment",
            sql,
            &["CommentStmt"],
            args.dry_run,
            false,
            &args.transaction,
        )
        .await
    })
}

pub fn routes() -> Result<Vec<Route>> {
    Ok(vec![
        route::<TypeArgs, ResultSet, _>(&tool_specs::PG_TYPE, TYPE_DESCRIPTION, type_tool)?,
        route::<ExtensionArgs, ResultSet, _>(
            &tool_specs::PG_EXTENSION,
            EXTENSION_DESCRIPTION,
            extension,
        )?,
        route::<CommentArgs, ResultSet, _>(&tool_specs::PG_COMMENT, COMMENT_DESCRIPTION, comment)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_that_reach_outside_the_database_are_refused() {
        for denied in DENIED_EXTENSIONS {
            assert!(
                refuse_dangerous_extension(denied).is_err(),
                "{denied} should be refused"
            );
            assert!(
                refuse_dangerous_extension(&denied.to_ascii_uppercase()).is_err(),
                "{denied} should be refused case-insensitively"
            );
        }
        for allowed in ["pgcrypto", "citext", "hstore", "uuid-ossp", "pg_trgm"] {
            assert!(
                refuse_dangerous_extension(allowed).is_ok(),
                "{allowed} should not be refused"
            );
        }
    }
}
