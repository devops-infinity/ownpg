use serde::Serialize;
use tokio_postgres::Row;

use crate::classify::CATALOG_SCHEMAS;
use crate::engine::Engine;
use crate::error::{Error, Result};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ObjectType {
    Schema,
    Table,
    View,
    MaterializedView,
    Sequence,
    Function,
    Procedure,
    Type,
    Index,
    Extension,
}

impl ObjectType {
    pub const ALL: [Self; 10] = [
        Self::Schema,
        Self::Table,
        Self::View,
        Self::MaterializedView,
        Self::Sequence,
        Self::Function,
        Self::Procedure,
        Self::Type,
        Self::Index,
        Self::Extension,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Schema => "schema",
            Self::Table => "table",
            Self::View => "view",
            Self::MaterializedView => "materialized_view",
            Self::Sequence => "sequence",
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Type => "type",
            Self::Index => "index",
            Self::Extension => "extension",
        }
    }

    #[must_use]
    pub fn from_relkind(kind: &str) -> Option<Self> {
        match kind {
            "r" | "p" | "f" => Some(Self::Table),
            "v" => Some(Self::View),
            "m" => Some(Self::MaterializedView),
            "S" => Some(Self::Sequence),
            "i" | "I" => Some(Self::Index),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRelation {
    pub schema: String,
    pub name: String,
    pub kind: ObjectType,
    pub relkind: String,
    pub oid: u32,
}

pub fn scope_check(schema: &str, scoped: &str) -> Result<()> {
    if schema == scoped || CATALOG_SCHEMAS.contains(&schema) {
        return Ok(());
    }
    Err(Error::StatementRefused {
        rule: format!(
            "object outside scoped schema: schema `{schema}` (this server serves `{scoped}`)"
        ),
        mode: "any".to_owned(),
    })
}

pub async fn resolve_relation(engine: &Engine, name: &str) -> Result<ResolvedRelation> {
    let rows = engine
        .catalog_rows(
            "SELECT n.nspname::text, c.relname::text, c.relkind::text, c.oid::int8 \
             FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.oid = pg_catalog.to_regclass($1::text)",
            &[&name],
        )
        .await?;
    let row = rows.first().ok_or_else(|| Error::ArgumentInvalid {
        argument: "name".to_owned(),
        detail: format!("`{name}` names no relation in the scoped schema"),
    })?;
    let schema: String = get(row, 0)?;
    let relname: String = get(row, 1)?;
    let relkind: String = get(row, 2)?;
    let oid: i64 = get(row, 3)?;
    scope_check(&schema, &engine.settings().schema.value)?;
    let kind = ObjectType::from_relkind(&relkind).ok_or_else(|| Error::ArgumentInvalid {
        argument: "name".to_owned(),
        detail: format!(
            "`{name}` has relation kind `{relkind}`, which this tool does not describe"
        ),
    })?;
    Ok(ResolvedRelation {
        schema,
        name: relname,
        kind,
        relkind,
        oid: u32::try_from(oid).unwrap_or(0),
    })
}

pub fn get<'a, T: tokio_postgres::types::FromSql<'a>>(row: &'a Row, index: usize) -> Result<T> {
    row.try_get(index).map_err(|error| Error::ProtocolFailed {
        detail: format!("catalog column {index} could not be read: {error}"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    pub not_null: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ConstraintInfo {
    pub name: String,
    pub kind: String,
    pub definition: String,
    pub validated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct IndexInfo {
    pub name: String,
    pub primary: bool,
    pub unique: bool,
    pub valid: bool,
    pub definition: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct TriggerInfo {
    pub name: String,
    pub definition: String,
    pub enabled: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct PolicyInfo {
    pub name: String,
    pub command: String,
    pub permissive: bool,
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub using: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_check: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct PartitionInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct TableDescription {
    pub schema: String,
    pub name: String,
    pub kind: ObjectType,
    pub persistence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub estimated_rows: i64,
    pub total_size_bytes: i64,
    pub table_size_bytes: i64,
    pub row_security: bool,
    pub columns: Vec<ColumnInfo>,
    pub constraints: Vec<ConstraintInfo>,
    pub indexes: Vec<IndexInfo>,
    pub triggers: Vec<TriggerInfo>,
    pub policies: Vec<PolicyInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_key: Option<String>,
    pub partitions: Vec<PartitionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_definition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub populated: Option<bool>,
}

pub async fn describe_relation(
    engine: &Engine,
    relation: &ResolvedRelation,
) -> Result<TableDescription> {
    let head = engine
        .catalog_rows(
            "SELECT c.relpersistence::text, pg_catalog.obj_description(c.oid, 'pg_class'), c.reltuples::float8, \
             pg_catalog.pg_total_relation_size(c.oid), pg_catalog.pg_relation_size(c.oid), c.relrowsecurity, \
             CASE WHEN c.relkind = 'p' THEN pg_catalog.pg_get_partkeydef(c.oid) END, \
             CASE WHEN c.relkind IN ('v', 'm') THEN pg_catalog.pg_get_viewdef(c.oid, true) END, \
             CASE WHEN c.relkind = 'm' THEN (SELECT ispopulated FROM pg_catalog.pg_matviews WHERE schemaname = $2 AND matviewname = c.relname) END \
             FROM pg_catalog.pg_class c WHERE c.oid = $1::oid",
            &[&relation.oid, &relation.schema],
        )
        .await?;
    let head = head.first().ok_or_else(|| Error::ArgumentInvalid {
        argument: "name".to_owned(),
        detail: "the relation vanished while it was being described".to_owned(),
    })?;
    let persistence: String = get(head, 0)?;
    let comment: Option<String> = get(head, 1)?;
    let reltuples: f64 = get(head, 2)?;
    let total_size: i64 = get(head, 3)?;
    let table_size: i64 = get(head, 4)?;
    let row_security: bool = get(head, 5)?;
    let partition_key: Option<String> = get(head, 6)?;
    let view_definition: Option<String> = get(head, 7)?;
    let populated: Option<bool> = get(head, 8)?;

    let mut columns = Vec::new();
    for row in engine
        .catalog_rows(
            "SELECT a.attname::text, pg_catalog.format_type(a.atttypid, a.atttypmod), \
             pg_catalog.pg_get_expr(d.adbin, d.adrelid, true), a.attnotnull, a.attidentity::text, a.attgenerated::text, \
             pg_catalog.col_description(a.attrelid, a.attnum) \
             FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON a.attrelid = d.adrelid AND a.attnum = d.adnum \
             WHERE a.attrelid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
            &[&relation.oid],
        )
        .await?
    {
        let identity: String = get(&row, 4)?;
        let generated: String = get(&row, 5)?;
        columns.push(ColumnInfo {
            name: get(&row, 0)?,
            data_type: get(&row, 1)?,
            default: get(&row, 2)?,
            not_null: get(&row, 3)?,
            identity: match identity.as_str() {
                "a" => Some("always".to_owned()),
                "d" => Some("by default".to_owned()),
                _ => None,
            },
            generated: match generated.as_str() {
                "s" => Some("stored".to_owned()),
                "v" => Some("virtual".to_owned()),
                _ => None,
            },
            comment: get(&row, 6)?,
        });
    }

    let mut constraints = Vec::new();
    for row in engine
        .catalog_rows(
            "SELECT conname::text, contype::text, pg_catalog.pg_get_constraintdef(oid, true), convalidated \
             FROM pg_catalog.pg_constraint WHERE conrelid = $1::oid ORDER BY contype, conname",
            &[&relation.oid],
        )
        .await?
    {
        let kind: String = get(&row, 1)?;
        constraints.push(ConstraintInfo {
            name: get(&row, 0)?,
            kind: match kind.as_str() {
                "p" => "primary key",
                "u" => "unique",
                "f" => "foreign key",
                "c" => "check",
                "x" => "exclusion",
                "n" => "not null",
                "t" => "trigger",
                _ => "other",
            }
            .to_owned(),
            definition: get(&row, 2)?,
            validated: get(&row, 3)?,
        });
    }

    let mut indexes = Vec::new();
    for row in engine
        .catalog_rows(
            "SELECT c2.relname::text, i.indisprimary, i.indisunique, i.indisvalid, \
             pg_catalog.pg_get_indexdef(i.indexrelid, 0, true), pg_catalog.pg_relation_size(i.indexrelid) \
             FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c2 ON c2.oid = i.indexrelid \
             WHERE i.indrelid = $1::oid ORDER BY i.indisprimary DESC, c2.relname",
            &[&relation.oid],
        )
        .await?
    {
        indexes.push(IndexInfo {
            name: get(&row, 0)?,
            primary: get(&row, 1)?,
            unique: get(&row, 2)?,
            valid: get(&row, 3)?,
            definition: get(&row, 4)?,
            size_bytes: get(&row, 5)?,
        });
    }

    let mut triggers = Vec::new();
    for row in engine
        .catalog_rows(
            "SELECT tgname::text, pg_catalog.pg_get_triggerdef(oid, true), tgenabled::text \
             FROM pg_catalog.pg_trigger WHERE tgrelid = $1::oid AND NOT tgisinternal ORDER BY tgname",
            &[&relation.oid],
        )
        .await?
    {
        let enabled: String = get(&row, 2)?;
        triggers.push(TriggerInfo {
            name: get(&row, 0)?,
            definition: get(&row, 1)?,
            enabled: match enabled.as_str() {
                "O" => "origin".to_owned(),
                "D" => "disabled".to_owned(),
                "R" => "replica".to_owned(),
                "A" => "always".to_owned(),
                other => other.to_owned(),
            },
        });
    }

    let mut policies = Vec::new();
    for row in engine
        .catalog_rows(
            "SELECT p.polname::text, p.polcmd::text, p.polpermissive, \
             COALESCE((SELECT pg_catalog.array_agg(r.rolname::text ORDER BY r.rolname) FROM pg_catalog.pg_roles r WHERE r.oid = ANY(p.polroles)), ARRAY[]::text[]), \
             pg_catalog.pg_get_expr(p.polqual, p.polrelid, true), pg_catalog.pg_get_expr(p.polwithcheck, p.polrelid, true) \
             FROM pg_catalog.pg_policy p WHERE p.polrelid = $1::oid ORDER BY p.polname",
            &[&relation.oid],
        )
        .await?
    {
        let command: String = get(&row, 1)?;
        policies.push(PolicyInfo {
            name: get(&row, 0)?,
            command: match command.as_str() {
                "r" => "select",
                "a" => "insert",
                "w" => "update",
                "d" => "delete",
                _ => "all",
            }
            .to_owned(),
            permissive: get(&row, 2)?,
            roles: get(&row, 3)?,
            using: get(&row, 4)?,
            with_check: get(&row, 5)?,
        });
    }

    let mut partitions = Vec::new();
    if relation.relkind == "p" {
        for row in engine
            .catalog_rows(
                "SELECT c.relname::text, pg_catalog.pg_get_expr(c.relpartbound, c.oid, true) \
                 FROM pg_catalog.pg_inherits i JOIN pg_catalog.pg_class c ON c.oid = i.inhrelid \
                 WHERE i.inhparent = $1::oid ORDER BY c.relname",
                &[&relation.oid],
            )
            .await?
        {
            partitions.push(PartitionInfo {
                name: get(&row, 0)?,
                bound: get(&row, 1)?,
            });
        }
    }

    Ok(TableDescription {
        schema: relation.schema.clone(),
        name: relation.name.clone(),
        kind: relation.kind,
        persistence: match persistence.as_str() {
            "p" => "permanent".to_owned(),
            "u" => "unlogged".to_owned(),
            "t" => "temporary".to_owned(),
            other => other.to_owned(),
        },
        comment,
        estimated_rows: reltuples.max(0.0).round() as i64,
        total_size_bytes: total_size,
        table_size_bytes: table_size,
        row_security,
        columns,
        constraints,
        indexes,
        triggers,
        policies,
        partition_key,
        partitions,
        view_definition,
        populated,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RoutineDescription {
    pub schema: String,
    pub name: String,
    pub signature: String,
    pub kind: String,
    pub language: String,
    pub security_definer: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    pub config: Vec<String>,
}

pub async fn describe_routines(engine: &Engine, name: &str) -> Result<Vec<RoutineDescription>> {
    let scoped = engine.settings().schema.value.clone();
    let (schema, bare) = split_name(name, &scoped);
    scope_check(&schema, &scoped)?;
    let mut out = Vec::new();
    for row in engine
        .catalog_rows(
            "SELECT n.nspname::text, p.proname::text, p.oid::regprocedure::text, p.prokind::text, l.lanname::text, p.prosecdef, \
             CASE WHEN p.prokind = 'p' THEN NULL ELSE pg_catalog.pg_get_function_result(p.oid) END, \
             pg_catalog.pg_get_function_arguments(p.oid), pg_catalog.obj_description(p.oid, 'pg_proc'), \
             CASE WHEN p.prokind IN ('f', 'p') THEN pg_catalog.pg_get_functiondef(p.oid) END, COALESCE(p.proconfig, ARRAY[]::text[]) \
             FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
             WHERE n.nspname = $1 AND p.proname = $2 ORDER BY p.oid",
            &[&schema, &bare],
        )
        .await?
    {
        let kind: String = get(&row, 3)?;
        out.push(RoutineDescription {
            schema: get(&row, 0)?,
            name: get(&row, 1)?,
            signature: get(&row, 2)?,
            kind: match kind.as_str() {
                "f" => "function",
                "p" => "procedure",
                "a" => "aggregate",
                "w" => "window",
                _ => "routine",
            }
            .to_owned(),
            language: get(&row, 4)?,
            security_definer: get(&row, 5)?,
            returns: get(&row, 6)?,
            arguments: get(&row, 7)?,
            comment: get(&row, 8)?,
            definition: get(&row, 9)?,
            config: get(&row, 10)?,
        });
    }
    if out.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "name".to_owned(),
            detail: format!("`{name}` names no function or procedure in schema `{schema}`"),
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct TypeDescription {
    pub schema: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub enum_labels: Vec<String>,
    pub attributes: Vec<ColumnInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_type: Option<String>,
    pub constraints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_subtype: Option<String>,
}

pub async fn describe_type(engine: &Engine, name: &str) -> Result<TypeDescription> {
    let scoped = engine.settings().schema.value.clone();
    let (schema, bare) = split_name(name, &scoped);
    scope_check(&schema, &scoped)?;
    let rows = engine
        .catalog_rows(
            "SELECT t.oid::int8, t.typtype::text, pg_catalog.obj_description(t.oid, 'pg_type'), \
             CASE WHEN t.typtype = 'd' THEN pg_catalog.format_type(t.typbasetype, t.typtypmod) END, \
             CASE WHEN t.typtype = 'r' THEN (SELECT pg_catalog.format_type(r.rngsubtype, NULL) FROM pg_catalog.pg_range r WHERE r.rngtypid = t.oid) END, \
             t.typrelid::int8 \
             FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
             WHERE n.nspname = $1 AND t.typname = $2",
            &[&schema, &bare],
        )
        .await?;
    let row = rows.first().ok_or_else(|| Error::ArgumentInvalid {
        argument: "name".to_owned(),
        detail: format!("`{name}` names no type in schema `{schema}`"),
    })?;
    let oid: i64 = get(row, 0)?;
    let typtype: String = get(row, 1)?;
    let comment: Option<String> = get(row, 2)?;
    let base_type: Option<String> = get(row, 3)?;
    let range_subtype: Option<String> = get(row, 4)?;
    let typrelid: i64 = get(row, 5)?;
    let oid_param = u32::try_from(oid).unwrap_or(0);
    let mut enum_labels = Vec::new();
    let mut attributes = Vec::new();
    let mut constraints = Vec::new();
    match typtype.as_str() {
        "e" => {
            for label in engine
                .catalog_rows(
                    "SELECT enumlabel::text FROM pg_catalog.pg_enum WHERE enumtypid = $1::oid ORDER BY enumsortorder",
                    &[&oid_param],
                )
                .await?
            {
                enum_labels.push(get(&label, 0)?);
            }
        }
        "c" => {
            for attribute in engine
                .catalog_rows(
                    "SELECT a.attname::text, pg_catalog.format_type(a.atttypid, a.atttypmod), pg_catalog.col_description(a.attrelid, a.attnum) \
                     FROM pg_catalog.pg_attribute a WHERE a.attrelid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
                    &[&u32::try_from(typrelid).unwrap_or(0)],
                )
                .await?
            {
                attributes.push(ColumnInfo {
                    name: get(&attribute, 0)?,
                    data_type: get(&attribute, 1)?,
                    default: None,
                    not_null: false,
                    identity: None,
                    generated: None,
                    comment: get(&attribute, 2)?,
                });
            }
        }
        "d" => {
            for constraint in engine
                .catalog_rows(
                    "SELECT pg_catalog.pg_get_constraintdef(oid, true) FROM pg_catalog.pg_constraint WHERE contypid = $1::oid ORDER BY conname",
                    &[&oid_param],
                )
                .await?
            {
                constraints.push(get(&constraint, 0)?);
            }
        }
        _ => {}
    }
    Ok(TypeDescription {
        schema,
        name: bare,
        kind: match typtype.as_str() {
            "e" => "enum",
            "c" => "composite",
            "d" => "domain",
            "r" => "range",
            "m" => "multirange",
            "b" => "base",
            _ => "other",
        }
        .to_owned(),
        comment,
        enum_labels,
        attributes,
        base_type,
        constraints,
        range_subtype,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct SequenceDescription {
    pub schema: String,
    pub name: String,
    pub data_type: String,
    pub start: i64,
    pub minimum: i64,
    pub maximum: i64,
    pub increment: i64,
    pub cycle: bool,
    pub cache: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
}

pub async fn describe_sequence(
    engine: &Engine,
    relation: &ResolvedRelation,
) -> Result<SequenceDescription> {
    let rows = engine
        .catalog_rows(
            "SELECT s.data_type::text, s.start_value, s.min_value, s.max_value, s.increment_by, s.cycle, s.cache_size, s.last_value, \
             (SELECT d.refobjid::regclass::text || '.' || a.attname FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
              WHERE d.objid = $3::oid AND d.deptype IN ('a', 'i') LIMIT 1) \
             FROM pg_catalog.pg_sequences s WHERE s.schemaname = $1 AND s.sequencename = $2",
            &[&relation.schema, &relation.name, &relation.oid],
        )
        .await?;
    let row = rows.first().ok_or_else(|| Error::ArgumentInvalid {
        argument: "name".to_owned(),
        detail: format!("`{}` names no sequence", relation.name),
    })?;
    Ok(SequenceDescription {
        schema: relation.schema.clone(),
        name: relation.name.clone(),
        data_type: get(row, 0)?,
        start: get(row, 1)?,
        minimum: get(row, 2)?,
        maximum: get(row, 3)?,
        increment: get(row, 4)?,
        cycle: get(row, 5)?,
        cache: get(row, 6)?,
        last_value: get(row, 7)?,
        owned_by: get(row, 8)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ExtensionDescription {
    pub name: String,
    pub version: String,
    pub schema: String,
    pub relocatable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

pub async fn describe_extension(engine: &Engine, name: &str) -> Result<ExtensionDescription> {
    let rows = engine
        .catalog_rows(
            "SELECT e.extname::text, e.extversion::text, n.nspname::text, e.extrelocatable, pg_catalog.obj_description(e.oid, 'pg_extension') \
             FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace WHERE e.extname = $1",
            &[&name],
        )
        .await?;
    let row = rows.first().ok_or_else(|| Error::ExtensionMissing {
        name: name.to_owned(),
    })?;
    Ok(ExtensionDescription {
        name: get(row, 0)?,
        version: get(row, 1)?,
        schema: get(row, 2)?,
        relocatable: get(row, 3)?,
        comment: get(row, 4)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RoleDescription {
    pub name: String,
    pub superuser: bool,
    pub inherit: bool,
    pub create_role: bool,
    pub create_db: bool,
    pub can_login: bool,
    pub replication: bool,
    pub bypass_rls: bool,
    pub connection_limit: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    pub member_of: Vec<String>,
    pub config: Vec<String>,
}

pub async fn describe_role(engine: &Engine, name: &str) -> Result<RoleDescription> {
    let rows = engine
        .catalog_rows(
            "SELECT r.rolname::text, r.rolsuper, r.rolinherit, r.rolcreaterole, r.rolcreatedb, r.rolcanlogin, r.rolreplication, r.rolbypassrls, r.rolconnlimit, \
             r.rolvaliduntil::text, \
             COALESCE((SELECT pg_catalog.array_agg(b.rolname::text ORDER BY b.rolname) FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles b ON b.oid = m.roleid WHERE m.member = r.oid), ARRAY[]::text[]), \
             COALESCE(r.rolconfig, ARRAY[]::text[]) \
             FROM pg_catalog.pg_roles r WHERE r.rolname = $1",
            &[&name],
        )
        .await?;
    let row = rows.first().ok_or_else(|| Error::ArgumentInvalid {
        argument: "name".to_owned(),
        detail: format!("`{name}` names no role"),
    })?;
    Ok(RoleDescription {
        name: get(row, 0)?,
        superuser: get(row, 1)?,
        inherit: get(row, 2)?,
        create_role: get(row, 3)?,
        create_db: get(row, 4)?,
        can_login: get(row, 5)?,
        replication: get(row, 6)?,
        bypass_rls: get(row, 7)?,
        connection_limit: get(row, 8)?,
        valid_until: get(row, 9)?,
        member_of: get(row, 10)?,
        config: get(row, 11)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct PrivilegeRow {
    pub grantee: String,
    pub privilege: String,
    pub grantable: bool,
    pub grantor: String,
}

pub async fn describe_privileges(
    engine: &Engine,
    relation: &ResolvedRelation,
) -> Result<Vec<PrivilegeRow>> {
    let mut out = Vec::new();
    for row in engine
        .catalog_rows(
            "SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.pg_get_userbyid(a.grantee)::text END, a.privilege_type::text, a.is_grantable, pg_catalog.pg_get_userbyid(a.grantor)::text \
             FROM pg_catalog.pg_class c, pg_catalog.aclexplode(COALESCE(c.relacl, pg_catalog.acldefault('r', c.relowner))) a \
             WHERE c.oid = $1::oid ORDER BY 1, 2",
            &[&relation.oid],
        )
        .await?
    {
        out.push(PrivilegeRow {
            grantee: get(&row, 0)?,
            privilege: get(&row, 1)?,
            grantable: get(&row, 2)?,
            grantor: get(&row, 3)?,
        });
    }
    Ok(out)
}

#[must_use]
pub fn split_name(name: &str, scoped: &str) -> (String, String) {
    let trimmed = name.trim();
    let unquote = |part: &str| -> String {
        let part = part.trim();
        if part.len() >= 2 && part.starts_with('"') && part.ends_with('"') {
            part.get(1..part.len() - 1)
                .unwrap_or(part)
                .replace("\"\"", "\"")
        } else {
            part.to_ascii_lowercase()
        }
    };
    if let Some((schema, bare)) = split_qualified(trimmed) {
        (unquote(schema), unquote(bare))
    } else {
        (scoped.to_owned(), unquote(trimmed))
    }
}

fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    for (index, c) in name.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '.' if !in_quotes => return Some((name.get(..index)?, name.get(index + 1..)?)),
            _ => {}
        }
    }
    None
}

#[must_use]
pub fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[must_use]
pub fn quote_identifier(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

impl ObjectType {
    #[must_use]
    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_on_the_unquoted_dot_and_fold_case_outside_quotes() {
        assert_eq!(
            split_name("orders", "app"),
            ("app".to_owned(), "orders".to_owned())
        );
        assert_eq!(
            split_name("Other.Orders", "app"),
            ("other".to_owned(), "orders".to_owned())
        );
        assert_eq!(
            split_name("\"Mixed.Case\".\"T\"", "app"),
            ("Mixed.Case".to_owned(), "T".to_owned())
        );
        assert_eq!(
            split_name("\"a\"\"b\"", "app"),
            ("app".to_owned(), "a\"b".to_owned())
        );
    }

    #[test]
    fn literals_and_identifiers_are_quoted_by_doubling() {
        assert_eq!(quote_literal("it's"), "'it''s'");
        assert_eq!(quote_identifier("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn the_scope_check_admits_the_catalogs_and_the_scoped_schema_only() {
        scope_check("app", "app").unwrap();
        scope_check("pg_catalog", "app").unwrap();
        scope_check("information_schema", "app").unwrap();
        let error = scope_check("other", "app").unwrap_err();
        assert!(error.to_string().contains("outside scoped schema"));
    }

    #[test]
    fn relkinds_map_onto_the_public_object_types() {
        assert_eq!(ObjectType::from_relkind("r"), Some(ObjectType::Table));
        assert_eq!(ObjectType::from_relkind("p"), Some(ObjectType::Table));
        assert_eq!(
            ObjectType::from_relkind("m"),
            Some(ObjectType::MaterializedView)
        );
        assert_eq!(ObjectType::from_relkind("S"), Some(ObjectType::Sequence));
        assert_eq!(ObjectType::from_relkind("c"), None);
    }
}
