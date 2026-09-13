use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::catalog;
use super::ddl::{Missing, Toggle, cascade_suffix, if_exists_clause, number, run_ddl, scoped_name};
use super::{AuditFacts, Call, Outcome, Route, ToolOutput, route, text_rows};
use crate::error::{Error, Result};
use crate::render::{expression, ident_list, quote_ident, quote_literal, validate_ident};
use crate::shape::{ResultSet, UNTRUSTED_NOTICE};
use crate::tool_specs;

const ROLE_DESCRIPTION: &str = "Create, alter, rename, or drop a role, grant or revoke membership in another role, and set or reset a configuration parameter for a role. Attributes cover login, createdb, createrole, inherit, replication, bypassrls, superuser, connection limit, password, and validity. The password never appears in logs or the audit trail. drop is destructive and needs confirm: true or the confirmation prompt.";

const GRANT_DESCRIPTION: &str = "Grant or revoke privileges on tables, sequences, routines, the scoped schema, the served database, or configuration parameters, including every table or sequence in the scoped schema, and set default privileges for objects created later. revoke can cascade to dependent grants.";

const POLICY_DESCRIPTION: &str = "Manage row-level security on a table in the scoped schema: create, alter, rename, or drop a policy, and enable, disable, force, or unforce row-level security on the table. A policy names the command it covers, the roles it applies to, whether it is permissive or restrictive, and its USING and WITH CHECK expressions.";

const PRIVILEGES_DESCRIPTION: &str = "List privileges, or apply a least-privilege template. object lists the grants on one table, view, or sequence; role lists what a role can do on every table in the scoped schema and which roles it belongs to; template applies the read_only, write_only, or read_write set to a role in one statement (schema usage, table and sequence privileges, default privileges, and the read-only session default), with dry_run showing the statements first.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoleOperation {
    Create,
    Alter,
    Rename,
    Drop,
    GrantMembership,
    RevokeMembership,
    SetConfig,
    ResetConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleArgs {
    pub operation: RoleOperation,
    #[schemars(description = "Role name.")]
    pub name: String,
    #[serde(default)]
    pub can_login: Toggle,
    #[serde(default)]
    pub superuser: Toggle,
    #[serde(default)]
    pub create_db: Toggle,
    #[serde(default)]
    pub create_role: Toggle,
    #[serde(default)]
    pub inherit: Toggle,
    #[serde(default)]
    pub replication: Toggle,
    #[serde(default)]
    pub bypass_rls: Toggle,
    #[serde(default)]
    #[schemars(description = "Whole number as text, -1 for no limit; empty leaves it unset.")]
    pub connection_limit: String,
    #[serde(default)]
    #[schemars(
        description = "create and alter: the password; it is sent to PostgreSQL and never logged."
    )]
    pub password: String,
    #[serde(default)]
    #[schemars(description = "alter: remove the password.")]
    pub clear_password: bool,
    #[serde(default)]
    #[schemars(
        description = "create and alter: a timestamp after which the password stops working, or infinity."
    )]
    pub valid_until: String,
    #[serde(default)]
    #[schemars(description = "create: roles the new role is a member of.")]
    pub in_roles: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "grant_membership and revoke_membership: the role to add the member to."
    )]
    pub group_role: String,
    #[serde(default)]
    #[schemars(description = "grant_membership: WITH ADMIN OPTION.")]
    pub admin_option: bool,
    #[serde(default)]
    #[schemars(description = "grant_membership: the INHERIT option (PostgreSQL 16 and later).")]
    pub inherit_option: Toggle,
    #[serde(default)]
    #[schemars(description = "grant_membership: the SET option (PostgreSQL 16 and later).")]
    pub set_option: Toggle,
    #[serde(default)]
    #[schemars(description = "set_config and reset_config: the parameter name.")]
    pub parameter: String,
    #[serde(default)]
    #[schemars(description = "set_config: the value.")]
    pub value: String,
    #[serde(default)]
    #[schemars(description = "rename: the new role name.")]
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

fn role_options(args: &RoleArgs) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let flag = |value: Toggle, yes: &str, no: &str| -> Option<String> {
        value
            .as_bool()
            .map(|on| if on { yes } else { no }.to_owned())
    };
    parts.extend(flag(args.can_login, "LOGIN", "NOLOGIN"));
    parts.extend(flag(args.superuser, "SUPERUSER", "NOSUPERUSER"));
    parts.extend(flag(args.create_db, "CREATEDB", "NOCREATEDB"));
    parts.extend(flag(args.create_role, "CREATEROLE", "NOCREATEROLE"));
    parts.extend(flag(args.inherit, "INHERIT", "NOINHERIT"));
    parts.extend(flag(args.replication, "REPLICATION", "NOREPLICATION"));
    parts.extend(flag(args.bypass_rls, "BYPASSRLS", "NOBYPASSRLS"));
    if let Some(limit) = number("connection_limit", &args.connection_limit)? {
        parts.push(format!("CONNECTION LIMIT {}", limit.max(-1)));
    }
    if args.clear_password {
        parts.push("PASSWORD NULL".to_owned());
    } else if !args.password.is_empty() {
        parts.push(format!("PASSWORD {}", quote_literal(&args.password)));
    }
    if !args.valid_until.trim().is_empty() {
        parts.push(format!(
            "VALID UNTIL {}",
            quote_literal(args.valid_until.trim())
        ));
    }
    Ok(parts)
}

pub fn role(call: Call, args: RoleArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        validate_ident("name", &args.name)?;
        let name = quote_ident(&args.name);
        let (sql, kinds): (String, &[&str]) = match args.operation {
            RoleOperation::Create => {
                let mut options = role_options(&args)?;
                if !args.in_roles.is_empty() {
                    options.push(format!(
                        "IN ROLE {}",
                        ident_list("in_roles", &args.in_roles)?
                    ));
                }
                (
                    format!(
                        "CREATE ROLE {name}{}{}",
                        if options.is_empty() { "" } else { " WITH " },
                        options.join(" ")
                    ),
                    &["CreateRoleStmt"],
                )
            }
            RoleOperation::Alter => {
                let options = role_options(&args)?;
                if options.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "operation".to_owned(),
                        detail: "alter needs at least one attribute to change".to_owned(),
                    }
                    .into());
                }
                (
                    format!("ALTER ROLE {name} WITH {}", options.join(" ")),
                    &["AlterRoleStmt"],
                )
            }
            RoleOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER ROLE {name} RENAME TO {}",
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            RoleOperation::Drop => (
                format!("DROP ROLE{} {name}", if_exists_clause(args.if_exists)),
                &["DropRoleStmt"],
            ),
            RoleOperation::GrantMembership | RoleOperation::RevokeMembership => {
                let mut missing = Missing::new();
                missing.need("group_role", !args.group_role.trim().is_empty());
                missing.finish("membership")?;
                validate_ident("group_role", args.group_role.trim())?;
                let target = quote_ident(args.group_role.trim());
                if args.operation == RoleOperation::GrantMembership {
                    let mut options = Vec::new();
                    if args.admin_option {
                        options.push("ADMIN OPTION".to_owned());
                    }
                    if let Some(inherit) = args.inherit_option.as_bool() {
                        options.push(format!("INHERIT {inherit}"));
                    }
                    if let Some(set) = args.set_option.as_bool() {
                        options.push(format!("SET {set}"));
                    }
                    (
                        format!(
                            "GRANT {target} TO {name}{}{}",
                            if options.is_empty() { "" } else { " WITH " },
                            options.join(", ")
                        ),
                        &["GrantRoleStmt"],
                    )
                } else {
                    let option = if args.admin_option {
                        "ADMIN OPTION FOR "
                    } else {
                        ""
                    };
                    (
                        format!("REVOKE {option}{target} FROM {name}"),
                        &["GrantRoleStmt"],
                    )
                }
            }
            RoleOperation::SetConfig => {
                let mut missing = Missing::new();
                missing
                    .need("parameter", !args.parameter.trim().is_empty())
                    .need("value", !args.value.trim().is_empty());
                missing.finish("set_config")?;
                validate_ident("parameter", args.parameter.trim())?;
                let value = args.value.trim();
                let rendered = if value.starts_with('\'') {
                    expression("value", value)?
                } else if value
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
                {
                    value.to_owned()
                } else {
                    quote_literal(value)
                };
                (
                    format!(
                        "ALTER ROLE {name} SET {} = {rendered}",
                        quote_ident(args.parameter.trim())
                    ),
                    &["AlterRoleSetStmt"],
                )
            }
            RoleOperation::ResetConfig => {
                let mut missing = Missing::new();
                missing.need("parameter", !args.parameter.trim().is_empty());
                missing.finish("reset_config")?;
                validate_ident("parameter", args.parameter.trim())?;
                (
                    format!(
                        "ALTER ROLE {name} RESET {}",
                        quote_ident(args.parameter.trim())
                    ),
                    &["AlterRoleSetStmt"],
                )
            }
        };
        let sql = if args.dry_run && !args.password.is_empty() && !args.clear_password {
            sql.replacen(
                &format!("PASSWORD {}", quote_literal(&args.password)),
                "PASSWORD '***'",
                1,
            )
        } else {
            sql
        };
        run_ddl(
            &call,
            "pg_role",
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
pub enum GrantOperation {
    Grant,
    Revoke,
    DefaultGrant,
    DefaultRevoke,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrantTarget {
    Table,
    AllTables,
    Sequence,
    AllSequences,
    Function,
    Procedure,
    AllRoutines,
    Schema,
    Database,
    Parameter,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantArgs {
    pub operation: GrantOperation,
    #[schemars(description = "What kind of object.")]
    pub target: GrantTarget,
    #[serde(default)]
    #[schemars(
        description = "Object names; empty for all_tables, all_sequences, all_routines, schema, and database."
    )]
    pub names: Vec<String>,
    #[schemars(description = "Privileges such as SELECT, INSERT, USAGE, EXECUTE, or ALL.")]
    pub privileges: Vec<String>,
    #[schemars(description = "Roles that receive or lose the privileges; PUBLIC is accepted.")]
    pub roles: Vec<String>,
    #[serde(default)]
    pub with_grant_option: bool,
    #[serde(default)]
    #[schemars(description = "revoke: also revoke dependent grants.")]
    pub cascade: bool,
    #[serde(default)]
    #[schemars(
        description = "default_grant and default_revoke: the role whose future objects are affected."
    )]
    pub for_role: String,
    #[serde(default)]
    #[schemars(description = "function and procedure: argument types that identify the overload.")]
    pub argument_types: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub transaction: String,
}

fn privilege_list(privileges: &[String], maintain_available: bool) -> Result<String> {
    if privileges.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "privileges".to_owned(),
            detail: "at least one privilege is required".to_owned(),
        });
    }
    let allowed = [
        "SELECT",
        "INSERT",
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "REFERENCES",
        "TRIGGER",
        "MAINTAIN",
        "USAGE",
        "EXECUTE",
        "CREATE",
        "CONNECT",
        "TEMPORARY",
        "TEMP",
        "SET",
        "ALTER SYSTEM",
        "ALL",
        "ALL PRIVILEGES",
    ];
    let mut out = Vec::new();
    for privilege in privileges {
        let upper = privilege.trim().to_ascii_uppercase();
        if !allowed.contains(&upper.as_str()) {
            return Err(Error::ArgumentInvalid {
                argument: "privileges".to_owned(),
                detail: format!("`{privilege}` is not a privilege name"),
            });
        }
        if upper == "MAINTAIN" && !maintain_available {
            return Err(Error::ArgumentInvalid {
                argument: "privileges".to_owned(),
                detail: "MAINTAIN exists from PostgreSQL 17 on; this server is older".to_owned(),
            });
        }
        out.push(upper);
    }
    Ok(out.join(", "))
}

fn role_list(roles: &[String]) -> Result<String> {
    if roles.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "roles".to_owned(),
            detail: "at least one role is required".to_owned(),
        });
    }
    let mut out = Vec::new();
    for role in roles {
        if role.trim().eq_ignore_ascii_case("public") {
            out.push("PUBLIC".to_owned());
        } else {
            validate_ident("roles", role.trim())?;
            out.push(quote_ident(role.trim()));
        }
    }
    Ok(out.join(", "))
}

pub fn grant(call: Call, args: GrantArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        let schema = quote_ident(&scoped);
        let privileges = privilege_list(
            &args.privileges,
            call.engine().features().supports_maintain_privilege(),
        )?;
        let roles = role_list(&args.roles)?;
        let named = |kind: &str| -> Result<String> {
            if args.names.is_empty() {
                return Err(Error::ArgumentInvalid {
                    argument: "names".to_owned(),
                    detail: format!("{kind} needs at least one name"),
                });
            }
            let names: Result<Vec<String>> = args
                .names
                .iter()
                .map(|name| scoped_name(&call, "names", name).map(|n| n.sql()))
                .collect();
            Ok(names?.join(", "))
        };
        let routine_signatures = |word: &str| -> Result<String> {
            if args.names.is_empty() {
                return Err(Error::ArgumentInvalid {
                    argument: "names".to_owned(),
                    detail: format!("{word} needs at least one name"),
                });
            }
            let types: Result<Vec<String>> = args
                .argument_types
                .iter()
                .map(|t| crate::render::type_name("argument_types", t))
                .collect();
            let types = types?.join(", ");
            let names: Result<Vec<String>> = args
                .names
                .iter()
                .map(|name| {
                    scoped_name(&call, "names", name).map(|n| format!("{}({types})", n.sql()))
                })
                .collect();
            Ok(names?.join(", "))
        };
        let object = match args.target {
            GrantTarget::Table => format!("TABLE {}", named("table")?),
            GrantTarget::AllTables => format!("ALL TABLES IN SCHEMA {schema}"),
            GrantTarget::Sequence => format!("SEQUENCE {}", named("sequence")?),
            GrantTarget::AllSequences => format!("ALL SEQUENCES IN SCHEMA {schema}"),
            GrantTarget::Function => format!("FUNCTION {}", routine_signatures("function")?),
            GrantTarget::Procedure => format!("PROCEDURE {}", routine_signatures("procedure")?),
            GrantTarget::AllRoutines => format!("ALL ROUTINES IN SCHEMA {schema}"),
            GrantTarget::Schema => format!("SCHEMA {schema}"),
            GrantTarget::Database => {
                format!("DATABASE {}", quote_ident(&call.settings().database.value))
            }
            GrantTarget::Parameter => {
                ident_list("names", &args.names)?;
                let parameters: Vec<String> =
                    args.names.iter().map(|p| quote_ident(p.trim())).collect();
                format!("PARAMETER {}", parameters.join(", "))
            }
            GrantTarget::Type => format!("TYPE {}", named("type")?),
        };
        let (sql, kinds): (String, &[&str]) = match args.operation {
            GrantOperation::Grant => (
                format!(
                    "GRANT {privileges} ON {object} TO {roles}{}",
                    if args.with_grant_option {
                        " WITH GRANT OPTION"
                    } else {
                        ""
                    }
                ),
                &["GrantStmt"],
            ),
            GrantOperation::Revoke => (
                format!(
                    "REVOKE {}{privileges} ON {object} FROM {roles}{}",
                    if args.with_grant_option {
                        "GRANT OPTION FOR "
                    } else {
                        ""
                    },
                    cascade_suffix(args.cascade)
                ),
                &["GrantStmt"],
            ),
            GrantOperation::DefaultGrant | GrantOperation::DefaultRevoke => {
                let kind = match args.target {
                    GrantTarget::Table | GrantTarget::AllTables => "TABLES",
                    GrantTarget::Sequence | GrantTarget::AllSequences => "SEQUENCES",
                    GrantTarget::Function | GrantTarget::Procedure | GrantTarget::AllRoutines => {
                        "ROUTINES"
                    }
                    GrantTarget::Type => "TYPES",
                    GrantTarget::Schema | GrantTarget::Database | GrantTarget::Parameter => {
                        return Err(Error::ArgumentInvalid {
                            argument: "target".to_owned(),
                            detail:
                                "default privileges cover tables, sequences, routines, and types"
                                    .to_owned(),
                        }
                        .into());
                    }
                };
                let for_role = if args.for_role.trim().is_empty() {
                    String::new()
                } else {
                    validate_ident("for_role", args.for_role.trim())?;
                    format!("FOR ROLE {} ", quote_ident(args.for_role.trim()))
                };
                let sql = if args.operation == GrantOperation::DefaultGrant {
                    format!(
                        "ALTER DEFAULT PRIVILEGES {for_role}IN SCHEMA {schema} GRANT {privileges} ON {kind} TO {roles}"
                    )
                } else {
                    format!(
                        "ALTER DEFAULT PRIVILEGES {for_role}IN SCHEMA {schema} REVOKE {privileges} ON {kind} FROM {roles}"
                    )
                };
                (sql, &["AlterDefaultPrivilegesStmt"])
            }
        };
        run_ddl(
            &call,
            "pg_grant",
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
pub enum PolicyOperation {
    Create,
    Alter,
    Rename,
    Drop,
    EnableRls,
    DisableRls,
    ForceRls,
    NoForceRls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyCommand {
    #[default]
    All,
    Select,
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyArgs {
    pub operation: PolicyOperation,
    #[schemars(description = "The table, optionally schema-qualified.")]
    pub table: String,
    #[serde(default)]
    #[schemars(description = "Policy name (not for the rls operations).")]
    pub name: String,
    #[serde(default)]
    pub command: PolicyCommand,
    #[serde(default)]
    #[schemars(description = "Roles the policy applies to; empty means PUBLIC.")]
    pub roles: Vec<String>,
    #[serde(default)]
    #[schemars(description = "create: a restrictive policy instead of a permissive one.")]
    pub restrictive: bool,
    #[serde(default)]
    #[schemars(description = "The USING expression.")]
    pub using: String,
    #[serde(default)]
    #[schemars(description = "The WITH CHECK expression.")]
    pub with_check: String,
    #[serde(default)]
    #[schemars(description = "rename: the new policy name.")]
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

pub fn policy(call: Call, args: PolicyArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let table = scoped_name(&call, "table", &args.table)?;
        let named = |field: &str| -> Result<String> {
            let mut missing = Missing::new();
            missing.need("name", !args.name.trim().is_empty());
            missing.finish(field)?;
            validate_ident("name", args.name.trim())?;
            Ok(quote_ident(args.name.trim()))
        };
        let clauses = |include_command: bool| -> Result<String> {
            let mut out = String::new();
            if include_command {
                out.push_str(match args.command {
                    PolicyCommand::All => " FOR ALL",
                    PolicyCommand::Select => " FOR SELECT",
                    PolicyCommand::Insert => " FOR INSERT",
                    PolicyCommand::Update => " FOR UPDATE",
                    PolicyCommand::Delete => " FOR DELETE",
                });
            }
            if !args.roles.is_empty() {
                out.push_str(&format!(" TO {}", role_list(&args.roles)?));
            }
            if !args.using.trim().is_empty() {
                out.push_str(&format!(" USING ({})", expression("using", &args.using)?));
            }
            if !args.with_check.trim().is_empty() {
                out.push_str(&format!(
                    " WITH CHECK ({})",
                    expression("with_check", &args.with_check)?
                ));
            }
            Ok(out)
        };
        let (sql, kinds): (String, &[&str]) = match args.operation {
            PolicyOperation::Create => (
                format!(
                    "CREATE POLICY {} ON {}{}{}",
                    named("create")?,
                    table.sql(),
                    if args.restrictive {
                        " AS RESTRICTIVE"
                    } else {
                        " AS PERMISSIVE"
                    },
                    clauses(true)?
                ),
                &["CreatePolicyStmt"],
            ),
            PolicyOperation::Alter => {
                let body = clauses(false)?;
                if body.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "operation".to_owned(),
                        detail: "alter needs roles, using, or with_check".to_owned(),
                    }
                    .into());
                }
                (
                    format!("ALTER POLICY {} ON {}{body}", named("alter")?, table.sql()),
                    &["AlterPolicyStmt"],
                )
            }
            PolicyOperation::Rename => {
                validate_ident("new_name", &args.new_name)?;
                (
                    format!(
                        "ALTER POLICY {} ON {} RENAME TO {}",
                        named("rename")?,
                        table.sql(),
                        quote_ident(&args.new_name)
                    ),
                    &["RenameStmt"],
                )
            }
            PolicyOperation::Drop => (
                format!(
                    "DROP POLICY{} {} ON {}",
                    if_exists_clause(args.if_exists),
                    named("drop")?,
                    table.sql()
                ),
                &["DropStmt"],
            ),
            PolicyOperation::EnableRls => (
                format!("ALTER TABLE {} ENABLE ROW LEVEL SECURITY", table.sql()),
                &["AlterTableStmt"],
            ),
            PolicyOperation::DisableRls => (
                format!("ALTER TABLE {} DISABLE ROW LEVEL SECURITY", table.sql()),
                &["AlterTableStmt"],
            ),
            PolicyOperation::ForceRls => (
                format!("ALTER TABLE {} FORCE ROW LEVEL SECURITY", table.sql()),
                &["AlterTableStmt"],
            ),
            PolicyOperation::NoForceRls => (
                format!("ALTER TABLE {} NO FORCE ROW LEVEL SECURITY", table.sql()),
                &["AlterTableStmt"],
            ),
        };
        run_ddl(
            &call,
            "pg_policy",
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
pub enum PrivilegesOperation {
    ListObject,
    ListRole,
    ApplyTemplate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Template {
    #[default]
    Unset,
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrivilegesArgs {
    pub operation: PrivilegesOperation,
    #[serde(default)]
    #[schemars(description = "object: the table, view, or sequence; role and template: the role.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "template: read_only, write_only, or read_write.")]
    pub template: Template,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub transaction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RolePrivilegeRow {
    pub table: String,
    pub privileges: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RolePrivileges {
    pub role: String,
    pub has_schema_usage: bool,
    pub member_of: Vec<String>,
    pub tables: Vec<RolePrivilegeRow>,
    pub notice: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ObjectPrivileges {
    pub object: String,
    pub rows: Vec<catalog::PrivilegeRow>,
    pub row_count: usize,
    pub notice: &'static str,
}

#[must_use]
pub fn template_statements(template: Template, role: &str, schema: &str) -> Vec<String> {
    let role = quote_ident(role);
    let schema = quote_ident(schema);
    let mut statements = vec![format!("GRANT USAGE ON SCHEMA {schema} TO {role}")];
    match template {
        Template::Unset | Template::ReadOnly => {
            statements.push(format!(
                "GRANT SELECT ON ALL TABLES IN SCHEMA {schema} TO {role}"
            ));
            statements.push(format!(
                "GRANT SELECT ON ALL SEQUENCES IN SCHEMA {schema} TO {role}"
            ));
            statements.push(format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT SELECT ON TABLES TO {role}"
            ));
            statements.push(format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT SELECT ON SEQUENCES TO {role}"
            ));
            statements.push(format!(
                "ALTER ROLE {role} SET default_transaction_read_only = on"
            ));
            statements.push(format!("ALTER ROLE {role} SET search_path = ''"));
            statements.push(format!("GRANT pg_read_all_stats TO {role}"));
        }
        Template::WriteOnly => {
            statements.push(format!(
                "GRANT INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO {role}"
            ));
            statements.push(format!(
                "GRANT USAGE ON ALL SEQUENCES IN SCHEMA {schema} TO {role}"
            ));
            statements.push(format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT INSERT, UPDATE, DELETE ON TABLES TO {role}"
            ));
            statements.push(format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT USAGE ON SEQUENCES TO {role}"
            ));
        }
        Template::ReadWrite => {
            statements.push(format!(
                "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO {role}"
            ));
            statements.push(format!(
                "GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA {schema} TO {role}"
            ));
            statements.push(format!(
                "GRANT EXECUTE ON ALL ROUTINES IN SCHEMA {schema} TO {role}"
            ));
            statements.push(format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO {role}"
            ));
            statements.push(format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT USAGE, SELECT ON SEQUENCES TO {role}"
            ));
            statements.push(format!(
                "ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT EXECUTE ON ROUTINES TO {role}"
            ));
        }
    }
    statements
}

#[must_use]
pub fn template_block(statements: &[String]) -> String {
    let body: Vec<String> = statements
        .iter()
        .map(|statement| format!("    EXECUTE {};", quote_literal(statement)))
        .collect();
    format!(
        "DO $ownpg_template$\nBEGIN\n{}\nEND\n$ownpg_template$",
        body.join("\n")
    )
}

pub fn privileges(call: Call, args: PrivilegesArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        match args.operation {
            PrivilegesOperation::ListObject => {
                let relation = catalog::resolve_relation(call.engine(), args.name.trim()).await?;
                let rows = catalog::describe_privileges(call.engine(), &relation).await?;
                let text = text_rows(
                    &[
                        ("grantee", "text"),
                        ("privilege", "text"),
                        ("grantable", "bool"),
                        ("grantor", "text"),
                    ],
                    rows.iter()
                        .map(|row| {
                            vec![
                                Some(row.grantee.clone()),
                                Some(row.privilege.clone()),
                                Some(row.grantable.to_string()),
                                Some(row.grantor.clone()),
                            ]
                        })
                        .collect(),
                    None,
                    None,
                );
                let result = ObjectPrivileges {
                    object: format!("{}.{}", relation.schema, relation.name),
                    row_count: rows.len(),
                    rows,
                    notice: UNTRUSTED_NOTICE,
                };
                Ok(ToolOutput::structured(&result, text)?
                    .with_facts(AuditFacts {
                        operation: Some("object".to_owned()),
                        row_count: Some(result.row_count as u64),
                        ..AuditFacts::default()
                    })
                    .into())
            }
            PrivilegesOperation::ListRole => {
                validate_ident("name", args.name.trim())?;
                let role = args.name.trim().to_owned();
                let profile = catalog::describe_role(call.engine(), &role).await?;
                let rows = call
                    .engine()
                    .catalog_rows(
                        "SELECT pg_catalog.has_schema_privilege($1, $2, 'USAGE')",
                        &[&role, &scoped],
                    )
                    .await?;
                let has_schema_usage: bool = rows
                    .first()
                    .map_or(Ok(false), |row| catalog::read_column(row, 0))?;
                let rows = call
                    .engine()
                    .catalog_rows(
                        "SELECT c.relname::text, \
                         ARRAY(SELECT p FROM unnest(ARRAY['SELECT','INSERT','UPDATE','DELETE','TRUNCATE','REFERENCES','TRIGGER']) AS p \
                               WHERE pg_catalog.has_table_privilege($1, c.oid, p)) \
                         FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $2 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') ORDER BY c.relname",
                        &[&role, &scoped],
                    )
                    .await?;
                let mut tables = Vec::new();
                for row in &rows {
                    tables.push(RolePrivilegeRow {
                        table: catalog::read_column(row, 0)?,
                        privileges: catalog::read_column(row, 1)?,
                    });
                }
                let text = text_rows(
                    &[("table", "text"), ("privileges", "text[]")],
                    tables
                        .iter()
                        .map(|row| vec![Some(row.table.clone()), Some(row.privileges.join(","))])
                        .collect(),
                    None,
                    None,
                );
                let result = RolePrivileges {
                    role,
                    has_schema_usage,
                    member_of: profile.member_of,
                    tables,
                    notice: UNTRUSTED_NOTICE,
                };
                let text = format!(
                    "role {} usage on {}: {}, member of: {}\n{}",
                    result.role,
                    scoped,
                    result.has_schema_usage,
                    if result.member_of.is_empty() {
                        "nothing".to_owned()
                    } else {
                        result.member_of.join(", ")
                    },
                    text
                );
                Ok(ToolOutput::structured(&result, text)?
                    .with_facts(AuditFacts {
                        operation: Some("role".to_owned()),
                        row_count: Some(result.tables.len() as u64),
                        ..AuditFacts::default()
                    })
                    .into())
            }
            PrivilegesOperation::ApplyTemplate => {
                if args.template == Template::Unset {
                    return Err(Error::ArgumentInvalid {
                        argument: "template".to_owned(),
                        detail: "template needs read_only, write_only, or read_write".to_owned(),
                    }
                    .into());
                }
                validate_ident("name", args.name.trim())?;
                let statements = template_statements(args.template, args.name.trim(), &scoped);
                let sql = template_block(&statements);
                run_ddl(
                    &call,
                    "pg_privileges",
                    sql,
                    &["DoStmt"],
                    args.dry_run,
                    false,
                    &args.transaction,
                )
                .await
            }
        }
    })
}

pub fn routes() -> Result<Vec<Route>> {
    Ok(vec![
        route::<RoleArgs, ResultSet, _>(&tool_specs::PG_ROLE, ROLE_DESCRIPTION, role)?,
        route::<GrantArgs, ResultSet, _>(&tool_specs::PG_GRANT, GRANT_DESCRIPTION, grant)?,
        route::<PolicyArgs, ResultSet, _>(&tool_specs::PG_POLICY, POLICY_DESCRIPTION, policy)?,
        route::<PrivilegesArgs, ResultSet, _>(
            &tool_specs::PG_PRIVILEGES,
            PRIVILEGES_DESCRIPTION,
            privileges,
        )?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privilege_and_role_lists_are_validated() {
        assert_eq!(
            privilege_list(&["select".to_owned(), "Insert".to_owned()], false).unwrap(),
            "SELECT, INSERT"
        );
        assert!(privilege_list(&["DROP".to_owned()], true).is_err());
        assert!(privilege_list(&["maintain".to_owned()], true).is_ok());
        let old = privilege_list(&["MAINTAIN".to_owned()], false).unwrap_err();
        assert!(old.to_string().contains("PostgreSQL 17"), "{old}");
        assert_eq!(
            role_list(&["public".to_owned(), "app".to_owned()]).unwrap(),
            "PUBLIC, \"app\""
        );
        assert!(role_list(&[]).is_err());
    }

    #[test]
    fn the_read_only_template_matches_the_documented_grants() {
        let statements = template_statements(Template::ReadOnly, "reader", "app");
        assert_eq!(statements.len(), 8);
        assert!(
            statements
                .iter()
                .any(|s| s.ends_with("SET search_path = ''"))
        );
        assert!(statements[0].contains("USAGE ON SCHEMA \"app\""));
        assert!(
            statements
                .iter()
                .any(|s| s.contains("default_transaction_read_only = on"))
        );
        assert!(statements.iter().any(|s| s.contains("pg_read_all_stats")));
        let block = template_block(&statements);
        assert!(block.starts_with("DO $ownpg_template$"));
        assert!(crate::render::verify(&block, &["DoStmt"]).is_ok());
    }
}
