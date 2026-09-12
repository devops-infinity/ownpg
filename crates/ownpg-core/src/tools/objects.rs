use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::catalog::{
    self, ExtensionDescription, ObjectType, PrivilegeRow, RoleDescription, RoutineDescription,
    SequenceDescription, TableDescription, TypeDescription,
};
use super::{AuditFacts, Call, Context, LIST_CAP, Outcome, Route, ToolOutput, route, text_rows};
use crate::error::Error;
use crate::shape::UNTRUSTED_NOTICE;
use crate::tool_specs;

const LIST_OBJECTS_DESCRIPTION: &str = "List objects in the scoped schema: tables, views, materialized views, sequences, functions, procedures, types, indexes, extensions, and the schemas of the database. Filter by object type and by a LIKE pattern on the name (% and _ are wildcards). The list is sorted by schema, then name, then OID, so repeated calls return the same order. At most 200 objects return per call; when more remain, truncated is true and cursor carries a token to pass back for the next page.";

const DESCRIBE_DESCRIPTION: &str = "Describe one object in detail. For a table, view, or materialized view: columns with types, defaults, identity and generated markers, constraints with validity, indexes with validity and size, triggers, row-level security policies, comments, size, and the estimated row count. Also describes sequences, functions and procedures (every overload), types (enum labels, composite attributes, domain constraints), extensions, roles, and the privileges granted on a relation. Names may be schema-qualified; unqualified names resolve in the scoped schema.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DetailLevel {
    Names,
    #[default]
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListObjectsArgs {
    #[serde(default)]
    #[schemars(description = "Object types to include. Empty means every type.")]
    pub object_types: Vec<ObjectType>,
    #[serde(default = "default_pattern")]
    #[schemars(description = "LIKE pattern on the object name. Default: % (every name).")]
    pub name_pattern: String,
    #[serde(default)]
    #[schemars(
        description = "names returns schema, name, and kind only; wants_summary adds owner, comment, size, and row estimate."
    )]
    pub detail_level: DetailLevel,
    #[serde(default)]
    #[schemars(
        description = "Cursor from a previous truncated result. Leave empty for the first page."
    )]
    pub cursor: String,
}

fn default_pattern() -> String {
    "%".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ObjectRow {
    pub schema: String,
    pub name: String,
    pub kind: ObjectType,
    pub oid: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_rows: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ObjectList {
    pub schema: String,
    pub rows: Vec<ObjectRow>,
    pub row_count: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub estimate: i64,
    pub order: &'static str,
    pub notice: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Keyset {
    #[serde(rename = "s")]
    schema: String,
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "o")]
    oid: i64,
}

fn encode_cursor(key: &Keyset) -> String {
    let json = serde_json::to_vec(key).unwrap_or_default();
    json.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_cursor(cursor: &str) -> Result<Keyset, Error> {
    let invalid = || Error::HandleState {
        handle: cursor.to_owned(),
        state: "not a cursor this tool issued".to_owned(),
    };
    if !cursor.len().is_multiple_of(2) || !cursor.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let bytes: Vec<u8> = (0..cursor.len())
        .step_by(2)
        .filter_map(|index| {
            cursor
                .get(index..index + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect();
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

const LIST_SQL: &str = "WITH objects AS ( \
SELECT n.nspname::text AS schema, c.relname::text AS name, c.oid::int8 AS oid, \
CASE c.relkind WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized_view' WHEN 'S' THEN 'sequence' WHEN 'i' THEN 'index' WHEN 'I' THEN 'index' ELSE 'table' END AS kind, \
pg_catalog.pg_get_userbyid(c.relowner)::text AS owner, pg_catalog.obj_description(c.oid, 'pg_class') AS comment, \
pg_catalog.pg_total_relation_size(c.oid) AS size_bytes, \
CASE WHEN c.relkind IN ('r', 'p', 'f', 'm') THEN GREATEST(c.reltuples, 0)::int8 END AS estimated_rows, \
CASE c.relkind WHEN 'p' THEN 'partitioned' WHEN 'f' THEN 'foreign' WHEN 'I' THEN 'partitioned index' \
WHEN 'r' THEN CASE c.relpersistence WHEN 'u' THEN 'unlogged' WHEN 't' THEN 'temporary' ELSE 'permanent' END END AS detail \
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm', 'S', 'i', 'I') \
UNION ALL \
SELECT n.nspname::text, p.proname::text, p.oid::int8, CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END, \
pg_catalog.pg_get_userbyid(p.proowner)::text, pg_catalog.obj_description(p.oid, 'pg_proc'), NULL::int8, NULL::int8, \
pg_catalog.pg_get_function_identity_arguments(p.oid) \
FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = $1 \
UNION ALL \
SELECT n.nspname::text, t.typname::text, t.oid::int8, 'type', pg_catalog.pg_get_userbyid(t.typowner)::text, \
pg_catalog.obj_description(t.oid, 'pg_type'), NULL::int8, NULL::int8, \
CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'c' THEN 'composite' WHEN 'd' THEN 'domain' WHEN 'r' THEN 'range' WHEN 'm' THEN 'multirange' ELSE 'base' END \
FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
WHERE n.nspname = $1 AND t.typtype IN ('e', 'c', 'd', 'r', 'm') \
AND (t.typrelid = 0 OR EXISTS (SELECT 1 FROM pg_catalog.pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c')) \
UNION ALL \
SELECT n.nspname::text, e.extname::text, e.oid::int8, 'extension', pg_catalog.pg_get_userbyid(e.extowner)::text, \
pg_catalog.obj_description(e.oid, 'pg_extension'), NULL::int8, NULL::int8, e.extversion::text \
FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
UNION ALL \
SELECT n.nspname::text, n.nspname::text, n.oid::int8, 'schema', pg_catalog.pg_get_userbyid(n.nspowner)::text, \
pg_catalog.obj_description(n.oid, 'pg_namespace'), NULL::int8, NULL::int8, CASE WHEN n.nspname = $1 THEN 'scoped' END \
FROM pg_catalog.pg_namespace n WHERE n.nspname NOT LIKE 'pg\\_%' AND n.nspname <> 'information_schema' \
) \
SELECT schema, name, kind, oid, owner, comment, size_bytes, estimated_rows, detail, count(*) OVER () AS total \
FROM objects \
WHERE kind = ANY($2::text[]) AND name LIKE $3 AND (schema, name, oid) > ($4::text, $5::text, $6::int8) \
ORDER BY schema, name, oid LIMIT $7::int8";

pub fn list_objects(call: Call, args: ListObjectsArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let context = call.context.clone();
        let scoped = context.settings().schema.value.clone();
        let kinds: Vec<String> = if args.object_types.is_empty() {
            ObjectType::ALL
                .iter()
                .map(|kind| kind.as_str().to_owned())
                .collect()
        } else {
            args.object_types
                .iter()
                .map(|kind| kind.as_str().to_owned())
                .collect()
        };
        let start = if args.cursor.is_empty() {
            Keyset {
                schema: String::new(),
                name: String::new(),
                oid: 0,
            }
        } else {
            decode_cursor(&args.cursor)?
        };
        let limit = i64::try_from(LIST_CAP + 1).unwrap_or(i64::MAX);
        let rows = context
            .engine
            .catalog_rows(
                LIST_SQL,
                &[
                    &scoped,
                    &kinds,
                    &args.name_pattern,
                    &start.schema,
                    &start.name,
                    &start.oid,
                    &limit,
                ],
            )
            .await?;
        let mut total: i64 = 0;
        let mut objects = Vec::new();
        for row in &rows {
            if objects.len() >= LIST_CAP {
                break;
            }
            total = catalog::read_column(row, 9)?;
            let kind_text: String = catalog::read_column(row, 2)?;
            let kind = ObjectType::parse(&kind_text).unwrap_or(ObjectType::Table);
            let wants_summary = args.detail_level == DetailLevel::Summary;
            objects.push(ObjectRow {
                schema: catalog::read_column(row, 0)?,
                name: catalog::read_column(row, 1)?,
                kind,
                oid: catalog::read_column(row, 3)?,
                owner: if wants_summary {
                    catalog::read_column(row, 4)?
                } else {
                    None
                },
                comment: if wants_summary {
                    catalog::read_column(row, 5)?
                } else {
                    None
                },
                size_bytes: if wants_summary {
                    catalog::read_column(row, 6)?
                } else {
                    None
                },
                estimated_rows: if wants_summary {
                    catalog::read_column(row, 7)?
                } else {
                    None
                },
                detail: if wants_summary {
                    catalog::read_column(row, 8)?
                } else {
                    None
                },
            });
        }
        let truncated = rows.len() > LIST_CAP;
        let cursor = if truncated {
            objects.last().map(|last| {
                encode_cursor(&Keyset {
                    schema: last.schema.clone(),
                    name: last.name.clone(),
                    oid: last.oid,
                })
            })
        } else {
            None
        };
        let text_lines: Vec<Vec<Option<String>>> = objects
            .iter()
            .map(|object| {
                vec![
                    Some(object.schema.clone()),
                    Some(object.name.clone()),
                    Some(object.kind.as_str().to_owned()),
                    object.detail.clone(),
                    object.size_bytes.map(|size| size.to_string()),
                    object.estimated_rows.map(|rows| rows.to_string()),
                    object.comment.clone(),
                ]
            })
            .collect();
        let text = text_rows(
            &[
                ("schema", "text"),
                ("name", "text"),
                ("kind", "text"),
                ("detail", "text"),
                ("size_bytes", "int8"),
                ("estimated_rows", "int8"),
                ("comment", "text"),
            ],
            text_lines,
            cursor.clone(),
            Some(total),
        );
        let facts = AuditFacts {
            operation: Some("list".to_owned()),
            row_count: Some(objects.len() as u64),
            truncated,
            handle_id: cursor.clone(),
            ..AuditFacts::default()
        };
        let list = ObjectList {
            schema: scoped,
            row_count: objects.len(),
            rows: objects,
            truncated,
            cursor,
            estimate: total,
            order: "schema, name, oid",
            notice: UNTRUSTED_NOTICE,
        };
        Ok(ToolOutput::structured(&list, text)?
            .with_facts(facts)
            .into())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DescribeTarget {
    #[default]
    Auto,
    Relation,
    Routine,
    Type,
    Extension,
    Role,
    Privileges,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeArgs {
    #[schemars(
        description = "Object name, optionally schema-qualified (schema.name). Quote mixed-case names with double quotes."
    )]
    pub name: String,
    #[serde(default)]
    #[schemars(
        description = "What the name refers to. auto tries a relation, then a routine, then a type, then an extension."
    )]
    pub target: DescribeTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ObjectDescription {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<TableDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<SequenceDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routines: Option<Vec<RoutineDescription>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_description: Option<TypeDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<RoleDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privileges: Option<Vec<PrivilegeRow>>,
    pub notice: &'static str,
}

impl ObjectDescription {
    fn empty(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            relation: None,
            sequence: None,
            routines: None,
            type_description: None,
            extension: None,
            role: None,
            privileges: None,
            notice: UNTRUSTED_NOTICE,
        }
    }
}

pub fn describe(call: Call, args: DescribeArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let context = call.context.clone();
        let engine = &context.engine;
        let name = args.name.trim();
        if name.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "name".to_owned(),
                detail: "an object name is required".to_owned(),
            }
            .into());
        }
        let description = match args.target {
            DescribeTarget::Auto => describe_auto(&context, name).await?,
            DescribeTarget::Relation => describe_relation_or_sequence(&context, name).await?,
            DescribeTarget::Routine => {
                let mut description = ObjectDescription::empty("routine");
                description.routines = Some(catalog::describe_routines(engine, name).await?);
                description
            }
            DescribeTarget::Type => {
                let mut description = ObjectDescription::empty("type");
                description.type_description = Some(catalog::describe_type(engine, name).await?);
                description
            }
            DescribeTarget::Extension => {
                let mut description = ObjectDescription::empty("extension");
                description.extension = Some(catalog::describe_extension(engine, name).await?);
                description
            }
            DescribeTarget::Role => {
                let mut description = ObjectDescription::empty("role");
                description.role = Some(catalog::describe_role(engine, name).await?);
                description
            }
            DescribeTarget::Privileges => {
                let relation = catalog::resolve_relation(engine, name).await?;
                let mut description = ObjectDescription::empty("privileges");
                description.privileges =
                    Some(catalog::describe_privileges(engine, &relation).await?);
                description
            }
        };
        let text = render_description(&description);
        let facts = AuditFacts {
            operation: Some(description.kind.clone()),
            ..AuditFacts::default()
        };
        Ok(ToolOutput::structured(&description, text)?
            .with_facts(facts)
            .into())
    })
}

async fn describe_relation_or_sequence(
    context: &Context,
    name: &str,
) -> Result<ObjectDescription, Error> {
    let engine = &context.engine;
    let relation = catalog::resolve_relation(engine, name).await?;
    if relation.kind == ObjectType::Sequence {
        let mut description = ObjectDescription::empty("sequence");
        description.sequence = Some(catalog::describe_sequence(engine, &relation).await?);
        return Ok(description);
    }
    let mut description = ObjectDescription::empty(relation.kind.as_str());
    description.relation = Some(catalog::describe_relation(engine, &relation).await?);
    Ok(description)
}

async fn describe_auto(context: &Context, name: &str) -> Result<ObjectDescription, Error> {
    let engine = &context.engine;
    match catalog::resolve_relation(engine, name).await {
        Ok(_) => return describe_relation_or_sequence(context, name).await,
        Err(Error::ArgumentInvalid { .. }) => {}
        Err(error) => return Err(error),
    }
    match catalog::describe_routines(engine, name).await {
        Ok(routines) => {
            let mut description = ObjectDescription::empty("routine");
            description.routines = Some(routines);
            return Ok(description);
        }
        Err(Error::ArgumentInvalid { .. }) => {}
        Err(error) => return Err(error),
    }
    match catalog::describe_type(engine, name).await {
        Ok(type_description) => {
            let mut description = ObjectDescription::empty("type");
            description.type_description = Some(type_description);
            return Ok(description);
        }
        Err(Error::ArgumentInvalid { .. }) => {}
        Err(error) => return Err(error),
    }
    match catalog::describe_extension(engine, name).await {
        Ok(extension) => {
            let mut description = ObjectDescription::empty("extension");
            description.extension = Some(extension);
            Ok(description)
        }
        Err(Error::ExtensionMissing { .. }) => Err(Error::ArgumentInvalid {
            argument: "name".to_owned(),
            detail: format!(
                "`{name}` names no relation, routine, type, or extension in schema `{}`",
                engine.settings().schema.value
            ),
        }),
        Err(error) => Err(error),
    }
}

fn render_description(description: &ObjectDescription) -> String {
    let mut out = String::new();
    out.push_str(UNTRUSTED_NOTICE);
    out.push('\n');
    if let Some(relation) = &description.relation {
        out.push_str(&format!(
            "{} {}.{} ({}, about {} rows, {} bytes total)\n",
            relation.kind.as_str(),
            relation.schema,
            relation.name,
            relation.persistence,
            relation.estimated_rows,
            relation.total_size_bytes
        ));
        if let Some(comment) = &relation.comment {
            out.push_str(&format!("comment: {comment}\n"));
        }
        for column in &relation.columns {
            let mut line = format!("  column {} {}", column.name, column.data_type);
            if column.not_null {
                line.push_str(" not null");
            }
            if let Some(default) = &column.default {
                line.push_str(&format!(" default {default}"));
            }
            if let Some(identity) = &column.identity {
                line.push_str(&format!(" identity {identity}"));
            }
            if let Some(generated) = &column.generated {
                line.push_str(&format!(" generated {generated}"));
            }
            if let Some(comment) = &column.comment {
                line.push_str(&format!(" ({comment})"));
            }
            out.push_str(&line);
            out.push('\n');
        }
        for constraint in &relation.constraints {
            out.push_str(&format!(
                "  constraint {} {}: {}{}\n",
                constraint.name,
                constraint.kind,
                constraint.definition,
                if constraint.validated {
                    ""
                } else {
                    " (not validated)"
                }
            ));
        }
        for index in &relation.indexes {
            out.push_str(&format!(
                "  index {} ({} bytes){}: {}\n",
                index.name,
                index.size_bytes,
                if index.valid { "" } else { " INVALID" },
                index.definition
            ));
        }
        for trigger in &relation.triggers {
            out.push_str(&format!(
                "  trigger {} [{}]: {}\n",
                trigger.name, trigger.enable_mode, trigger.definition
            ));
        }
        for policy in &relation.policies {
            out.push_str(&format!(
                "  policy {} for {} ({}) roles {}\n",
                policy.name,
                policy.command,
                if policy.permissive {
                    "permissive"
                } else {
                    "restrictive"
                },
                if policy.roles.is_empty() {
                    "public".to_owned()
                } else {
                    policy.roles.join(", ")
                }
            ));
        }
        if let Some(key) = &relation.partition_key {
            out.push_str(&format!("  partition key: {key}\n"));
            for partition in &relation.partitions {
                out.push_str(&format!(
                    "  partition {}: {}\n",
                    partition.name,
                    partition.bound.as_deref().unwrap_or("default")
                ));
            }
        }
        if let Some(definition) = &relation.view_definition {
            out.push_str("  definition:\n");
            out.push_str(definition);
            out.push('\n');
        }
        if let Some(populated) = relation.populated {
            out.push_str(&format!("  populated: {populated}\n"));
        }
        return out;
    }
    if let Some(sequence) = &description.sequence {
        out.push_str(&format!(
            "sequence {}.{} {} start {} min {} max {} increment {} cache {}{}\n",
            sequence.schema,
            sequence.name,
            sequence.data_type,
            sequence.start,
            sequence.min_value,
            sequence.max_value,
            sequence.increment,
            sequence.cache,
            if sequence.cycle { " cycle" } else { "" }
        ));
        if let Some(last) = sequence.last_value {
            out.push_str(&format!("  last value: {last}\n"));
        }
        if let Some(owner) = &sequence.owned_by {
            out.push_str(&format!("  owned by: {owner}\n"));
        }
        return out;
    }
    if let Some(routines) = &description.routines {
        for routine in routines {
            out.push_str(&format!(
                "{} {} language {}{}{}\n",
                routine.kind,
                routine.signature,
                routine.language,
                routine
                    .returns
                    .as_ref()
                    .map_or(String::new(), |returns| format!(" returns {returns}")),
                if routine.security_definer {
                    " security definer"
                } else {
                    ""
                }
            ));
            if let Some(comment) = &routine.comment {
                out.push_str(&format!("  comment: {comment}\n"));
            }
            for setting in &routine.config {
                out.push_str(&format!("  set {setting}\n"));
            }
            if let Some(definition) = &routine.definition {
                out.push_str(definition);
                if !definition.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        return out;
    }
    if let Some(type_description) = &description.type_description {
        out.push_str(&format!(
            "type {}.{} ({})\n",
            type_description.schema, type_description.name, type_description.kind
        ));
        if let Some(comment) = &type_description.comment {
            out.push_str(&format!("  comment: {comment}\n"));
        }
        if !type_description.enum_labels.is_empty() {
            out.push_str(&format!(
                "  labels: {}\n",
                type_description.enum_labels.join(", ")
            ));
        }
        for attribute in &type_description.attributes {
            out.push_str(&format!(
                "  attribute {} {}\n",
                attribute.name, attribute.data_type
            ));
        }
        if let Some(base) = &type_description.base_type {
            out.push_str(&format!("  base type: {base}\n"));
        }
        for constraint in &type_description.constraints {
            out.push_str(&format!("  constraint: {constraint}\n"));
        }
        if let Some(subtype) = &type_description.range_subtype {
            out.push_str(&format!("  range of: {subtype}\n"));
        }
        return out;
    }
    if let Some(extension) = &description.extension {
        out.push_str(&format!(
            "extension {} version {} in schema {}{}\n",
            extension.name,
            extension.version,
            extension.schema,
            if extension.relocatable {
                " (relocatable)"
            } else {
                ""
            }
        ));
        if let Some(comment) = &extension.comment {
            out.push_str(&format!("  comment: {comment}\n"));
        }
        return out;
    }
    if let Some(role) = &description.role {
        let mut attributes = Vec::new();
        if role.superuser {
            attributes.push("superuser");
        }
        if role.create_db {
            attributes.push("createdb");
        }
        if role.create_role {
            attributes.push("createrole");
        }
        if role.can_login {
            attributes.push("login");
        }
        if role.replication {
            attributes.push("replication");
        }
        if role.bypass_rls {
            attributes.push("bypassrls");
        }
        if !role.inherit {
            attributes.push("noinherit");
        }
        out.push_str(&format!(
            "role {} [{}] connection limit {}\n",
            role.name,
            attributes.join(", "),
            role.connection_limit
        ));
        if let Some(until) = &role.valid_until {
            out.push_str(&format!("  valid until: {until}\n"));
        }
        if !role.member_of.is_empty() {
            out.push_str(&format!("  member of: {}\n", role.member_of.join(", ")));
        }
        for setting in &role.config {
            out.push_str(&format!("  set {setting}\n"));
        }
        return out;
    }
    if let Some(privileges) = &description.privileges {
        for grant in privileges {
            out.push_str(&format!(
                "  {} has {}{} (granted by {})\n",
                grant.grantee,
                grant.privilege,
                if grant.grantable {
                    " with grant option"
                } else {
                    ""
                },
                grant.grantor
            ));
        }
    }
    out
}

pub fn routes() -> Result<Vec<Route>, Error> {
    Ok(vec![
        route::<ListObjectsArgs, ObjectList, _>(
            &tool_specs::PG_LIST_OBJECTS,
            LIST_OBJECTS_DESCRIPTION,
            list_objects,
        )?,
        route::<DescribeArgs, ObjectDescription, _>(
            &tool_specs::PG_DESCRIBE,
            DESCRIBE_DESCRIPTION,
            describe,
        )?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyset_cursor_round_trips_and_garbage_is_refused() {
        let key = Keyset {
            schema: "app".to_owned(),
            name: "orders".to_owned(),
            oid: 16_384,
        };
        let cursor = encode_cursor(&key);
        assert!(cursor.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(decode_cursor(&cursor).unwrap(), key);
        assert!(decode_cursor("zz").is_err());
        assert!(decode_cursor("abc").is_err());
        assert!(decode_cursor("7b7d").is_err());
    }

    #[test]
    fn the_argument_defaults_read_every_type_with_a_summary() {
        let args: ListObjectsArgs = serde_json::from_str("{}").unwrap();
        assert!(args.object_types.is_empty());
        assert_eq!(args.name_pattern, "%");
        assert_eq!(args.detail_level, DetailLevel::Summary);
        assert!(args.cursor.is_empty());
        let rejected = serde_json::from_str::<ListObjectsArgs>("{\"schema\": \"x\"}");
        assert!(rejected.is_err());
    }
}
