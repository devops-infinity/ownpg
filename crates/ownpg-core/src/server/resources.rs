use std::fmt::Write as _;

use rmcp::model::{
    CacheScope, ErrorData, ListResourceTemplatesResult, ListResourcesResult, ReadResourceResult,
    Resource, ResourceContents, ResourceTemplate,
};

use super::{LIST_TTL_MS, Server};
use crate::error::Error;
use crate::tools::objects::{DescribeArgs, DescribeKind, DetailLevel, ListObjectsArgs, describe};
use crate::tools::{Call, Reply, ToolFailure};

pub const SCHEMA_TEMPLATE: &str = "postgres://{database}/{schema}";
pub const TABLE_TEMPLATE: &str = "postgres://{database}/{schema}/{table}";
pub const MIME_TYPE: &str = "application/json";
pub const LIST_CAP: usize = 1_000;

const RELATIONS_SQL: &str = "SELECT c.relname::text, c.relkind::text, pg_catalog.obj_description(c.oid, 'pg_class') \
     FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') ORDER BY c.relname LIMIT $2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Schema,
    Table(String),
}

#[must_use]
pub fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn decode(segment: &str) -> Option<String> {
    let mut out = Vec::with_capacity(segment.len());
    let mut bytes = segment.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let pair = [bytes.next()?, bytes.next()?];
            let pair = std::str::from_utf8(&pair).ok()?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
        } else {
            out.push(byte);
        }
    }
    String::from_utf8(out).ok()
}

#[must_use]
pub fn schema_uri(database: &str, schema: &str) -> String {
    format!("postgres://{}/{}", encode(database), encode(schema))
}

#[must_use]
pub fn table_uri(database: &str, schema: &str, table: &str) -> String {
    format!(
        "postgres://{}/{}/{}",
        encode(database),
        encode(schema),
        encode(table)
    )
}

pub fn parse_uri(uri: &str, database: &str, schema: &str) -> Result<Target, ErrorData> {
    let not_found = |detail: &str| {
        ErrorData::resource_not_found(
            format!("{uri} is not a resource of this server: {detail}"),
            None,
        )
    };
    let rest = uri
        .strip_prefix("postgres://")
        .ok_or_else(|| not_found("the scheme must be postgres://"))?;
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.len() != 2 && segments.len() != 3 {
        return Err(not_found(
            "the path is {database}/{schema} or {database}/{schema}/{table}",
        ));
    }
    let decoded: Option<Vec<String>> = segments.iter().map(|segment| decode(segment)).collect();
    let decoded =
        decoded.ok_or_else(|| not_found("a path segment is not valid percent encoding"))?;
    let mut decoded = decoded.into_iter();
    if decoded.next().as_deref() != Some(database) {
        return Err(not_found(&format!(
            "this server is connected to database {database}"
        )));
    }
    if decoded.next().as_deref() != Some(schema) {
        return Err(not_found(&format!(
            "this server serves schema {schema} only"
        )));
    }
    match decoded.next() {
        None => Ok(Target::Schema),
        Some(table) if table.is_empty() => Err(not_found("the table segment is empty")),
        Some(table) => Ok(Target::Table(table)),
    }
}

#[must_use]
pub fn templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new(SCHEMA_TEMPLATE, "schema")
            .with_title("Schema listing")
            .with_description(
                "Every object in the scoped schema with its kind, owner, comment, size, and row estimate; the same JSON pg_list_objects returns.",
            )
            .with_mime_type(MIME_TYPE),
        ResourceTemplate::new(TABLE_TEMPLATE, "table")
            .with_title("Table description")
            .with_description(
                "Columns, constraints, indexes, triggers, and policies of one table, view, or materialized view in the scoped schema; the same JSON pg_describe returns.",
            )
            .with_mime_type(MIME_TYPE),
    ]
}

#[must_use]
pub fn list_templates() -> ListResourceTemplatesResult {
    ListResourceTemplatesResult::with_all_items(templates())
        .with_ttl_ms(LIST_TTL_MS)
        .with_cache_scope(CacheScope::Private)
}

impl Server {
    fn resource_call(&self) -> Call {
        Call {
            context: self.context.clone(),
            arguments: rmcp::model::JsonObject::new(),
            principal: self.principal.name.clone(),
            request_state: None,
            input_responses: None,
            elicitation: false,
            progress: None,
        }
    }

    #[must_use]
    pub fn schema_resource_uri(&self) -> String {
        let settings = self.context.settings();
        schema_uri(&settings.database.value, &settings.schema.value)
    }

    #[must_use]
    pub fn table_resource_uri(&self, table: &str) -> String {
        let settings = self.context.settings();
        table_uri(&settings.database.value, &settings.schema.value, table)
    }

    pub async fn list_resource_items(&self) -> Result<ListResourcesResult, ErrorData> {
        let settings = self.context.settings();
        let schema = settings.schema.value.clone();
        let mut items = vec![
            Resource::new(self.schema_resource_uri(), schema.clone())
                .with_title(format!("Schema {schema}"))
                .with_description("Every object in the scoped schema.")
                .with_mime_type(MIME_TYPE),
        ];
        let limit = i64::try_from(LIST_CAP).unwrap_or(i64::MAX);
        let rows = self
            .context
            .engine
            .catalog_rows(RELATIONS_SQL, &[&schema, &limit])
            .await
            .map_err(resource_error)?;
        for row in &rows {
            let name: String = crate::tools::catalog::get(row, 0).map_err(resource_error)?;
            let kind: String = crate::tools::catalog::get(row, 1).map_err(resource_error)?;
            let comment: Option<String> =
                crate::tools::catalog::get(row, 2).map_err(resource_error)?;
            let kind_label = match kind.as_str() {
                "v" => "view",
                "m" => "materialized view",
                "f" => "foreign table",
                "p" => "partitioned table",
                _ => "table",
            };
            let mut resource = Resource::new(self.table_resource_uri(&name), name.clone())
                .with_title(format!("{kind_label} {schema}.{name}"))
                .with_mime_type(MIME_TYPE);
            if let Some(comment) = comment {
                resource = resource.with_description(comment);
            }
            items.push(resource);
        }
        Ok(ListResourcesResult::with_all_items(items)
            .with_ttl_ms(LIST_TTL_MS)
            .with_cache_scope(CacheScope::Private))
    }

    pub async fn read_resource_item(&self, uri: &str) -> Result<ReadResourceResult, ErrorData> {
        let settings = self.context.settings();
        let target = parse_uri(uri, &settings.database.value, &settings.schema.value)?;
        let call = self.resource_call();
        let outcome = match &target {
            Target::Schema => {
                crate::tools::objects::list_objects(
                    call,
                    ListObjectsArgs {
                        object_types: Vec::new(),
                        name_pattern: "%".to_owned(),
                        detail_level: DetailLevel::Summary,
                        cursor: String::new(),
                    },
                )
                .await
            }
            Target::Table(table) => {
                describe(
                    call,
                    DescribeArgs {
                        name: table.clone(),
                        object_type: DescribeKind::Relation,
                    },
                )
                .await
            }
        };
        let output = match outcome {
            Ok(Reply::Output(output)) => output,
            Ok(Reply::InputRequired(_)) => {
                return Err(ErrorData::internal_error(
                    "a resource read never asks for input",
                    None,
                ));
            }
            Err(failure) => return Err(failure_error(uri, &failure)),
        };
        let text = serde_json::to_string_pretty(&output.structured).map_err(|error| {
            ErrorData::internal_error(
                format!("the resource could not be serialized: {error}"),
                None,
            )
        })?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, uri.to_owned()).with_mime_type(MIME_TYPE),
        ])
        .with_ttl_ms(LIST_TTL_MS)
        .with_cache_scope(CacheScope::Private))
    }
}

fn resource_error(error: Error) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

fn failure_error(uri: &str, failure: &ToolFailure) -> ErrorData {
    match failure.error() {
        Error::ArgumentInvalid { detail, .. } => {
            ErrorData::resource_not_found(format!("{uri}: {detail}"), None)
        }
        error => ErrorData::internal_error(error.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip_through_percent_encoding() {
        let uri = table_uri("app db", "sch/ema", "Order Items");
        assert_eq!(uri, "postgres://app%20db/sch%2Fema/Order%20Items");
        assert_eq!(
            parse_uri(&uri, "app db", "sch/ema").unwrap(),
            Target::Table("Order Items".to_owned())
        );
        assert_eq!(
            parse_uri("postgres://app%20db/sch%2Fema", "app db", "sch/ema").unwrap(),
            Target::Schema
        );
    }

    #[test]
    fn foreign_databases_schemas_and_shapes_are_not_found() {
        for uri in [
            "postgres://other/app",
            "postgres://app/other",
            "postgres://app",
            "postgres://app/app/orders/extra",
            "postgres://app/app/",
            "postgres://app/app/%ZZ",
            "https://app/app",
        ] {
            let error = parse_uri(uri, "app", "app").unwrap_err();
            assert_eq!(
                error.code,
                rmcp::model::ErrorCode::RESOURCE_NOT_FOUND,
                "{uri}"
            );
        }
    }

    #[test]
    fn templates_carry_the_documented_shapes() {
        let templates = templates();
        assert_eq!(templates[0].uri_template, SCHEMA_TEMPLATE);
        assert_eq!(templates[1].uri_template, TABLE_TEMPLATE);
        let listed = list_templates();
        assert_eq!(listed.ttl_ms, Some(LIST_TTL_MS));
        assert_eq!(listed.cache_scope, Some(CacheScope::Private));
    }
}
