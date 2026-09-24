pub mod cascade;
pub mod catalog;
pub mod confirm;
pub mod ddl;
pub mod health;
pub mod host;
pub mod maintenance;
pub mod monitoring;
pub mod objects;
pub mod pooler;
pub mod read;
pub mod roles;
pub mod transaction;
pub mod write;

use std::sync::Arc;

use futures_util::future::BoxFuture;
use rmcp::handler::server::tool::{schema_for_input, schema_for_output};
use rmcp::model::{
    CallToolResult, ContentBlock, InputRequiredResult, InputResponses, JsonObject, Tool,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::audit::{Decision, Sink, Transport};
use crate::config::{MAX_ROW_CAP, ResultText, Settings};
use crate::engine::Engine;
use crate::error::Error;
use crate::shape::{Caps, ResultSet};
use crate::tool_specs::ToolSpec;

pub const LIST_CAP: usize = 200;

#[derive(Debug, Clone)]
pub struct Context {
    pub engine: Arc<Engine>,
    pub transport: Transport,
    pub audit: Arc<Sink>,
    pub gate: Arc<confirm::Gate>,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub peer: rmcp::service::Peer<rmcp::RoleServer>,
    pub token: rmcp::model::ProgressToken,
}

impl Progress {
    pub async fn report(&self, progress: f64, total: Option<f64>, message: String) {
        let mut param = rmcp::model::ProgressNotificationParam::new(self.token.clone(), progress);
        param.total = total;
        param.message = Some(message);
        if let Err(error) = self.peer.notify_progress(param).await {
            tracing::debug!(%error, "a progress notification was not delivered");
        }
    }
}

#[derive(Debug, Clone)]
pub struct Call {
    pub context: Context,
    pub arguments: JsonObject,
    pub principal: String,
    pub scopes: Option<Vec<String>>,
    pub request_state: Option<String>,
    pub input_responses: Option<InputResponses>,
    pub can_elicit: bool,
    pub progress: Option<Progress>,
    pub cancel: tokio_util::sync::CancellationToken,
}

impl Call {
    #[must_use]
    pub fn engine(&self) -> &Arc<Engine> {
        &self.context.engine
    }

    #[must_use]
    pub fn settings(&self) -> &Arc<Settings> {
        self.context.settings()
    }

    #[must_use]
    pub fn caps(&self, row_cap: u32) -> Caps {
        self.context.caps(row_cap)
    }

    #[must_use]
    pub fn allows(&self, scope: &str) -> bool {
        self.scopes
            .as_ref()
            .is_none_or(|held| held.iter().any(|granted| granted == scope))
    }
}

impl Context {
    #[must_use]
    pub fn settings(&self) -> &Arc<Settings> {
        self.engine.settings()
    }

    #[must_use]
    pub fn caps(&self, row_cap: u32) -> Caps {
        let limits = &self.settings().limits;
        let requested = if row_cap == 0 {
            limits.row_cap.value
        } else {
            row_cap.min(MAX_ROW_CAP)
        };
        Caps {
            row_cap: requested as usize,
            byte_cap: limits.byte_cap.value as usize,
            cell_cap: crate::shape::CELL_CAP_BYTES,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AuditFacts {
    pub operation: Option<String>,
    pub statement_class: Option<String>,
    pub statement_hash: Option<String>,
    pub statement: Option<String>,
    pub handle_id: Option<String>,
    pub cursor_id: Option<String>,
    pub row_count: Option<u64>,
    pub rows_affected: Option<u64>,
    pub truncated: bool,
    pub decision: Option<Decision>,
    pub relations: Vec<String>,
}

impl AuditFacts {
    #[must_use]
    pub fn with_result(mut self, result: &ResultSet) -> Self {
        self.row_count = Some(result.row_count as u64);
        self.rows_affected = result.rows_affected;
        self.truncated = result.truncated;
        if result.cursor.is_some() {
            self.cursor_id.clone_from(&result.cursor);
        }
        self
    }
}

#[derive(Debug)]
pub struct ToolOutput {
    pub text: String,
    pub structured: serde_json::Value,
    pub facts: AuditFacts,
}

fn sanitize_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            if text.chars().any(crate::shape::is_invisible) {
                *text = crate::shape::sanitize(text);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(sanitize_strings),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(sanitize_strings),
        _ => {}
    }
}

impl ToolOutput {
    pub fn structured<T: Serialize>(value: &T, text: String) -> Result<Self, ToolFailure> {
        let mut structured = serde_json::to_value(value).map_err(|error| {
            ToolFailure::from(Error::ProtocolFailed {
                detail: format!("the result could not be serialized: {error}"),
            })
        })?;
        sanitize_strings(&mut structured);
        Ok(Self {
            text: crate::shape::sanitize(&text),
            structured,
            facts: AuditFacts::default(),
        })
    }

    #[must_use]
    pub fn with_facts(mut self, facts: AuditFacts) -> Self {
        self.facts = facts;
        self
    }

    #[must_use]
    pub fn into_call_result(self, shape: ResultText) -> CallToolResult {
        let text = match shape {
            ResultText::Full => self.text,
            ResultText::Summary => summary_text(&self.structured, self.text),
        };
        let mut result = CallToolResult::structured(self.structured);
        result.content = vec![ContentBlock::text(text)];
        result
    }
}

const SUMMARY_KEEPS_TEXT_UP_TO: usize = 400;

fn summary_text(structured: &serde_json::Value, text: String) -> String {
    if text.len() <= SUMMARY_KEEPS_TEXT_UP_TO {
        return text;
    }
    let mut parts = Vec::new();
    if let Some(rows) = structured
        .get("row_count")
        .and_then(serde_json::Value::as_u64)
    {
        parts.push(if rows == 1 {
            "1 row".to_owned()
        } else {
            format!("{rows} rows")
        });
    }
    if structured
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        parts.push("more remain".to_owned());
    }
    if let Some(cursor) = structured.get("cursor").and_then(serde_json::Value::as_str) {
        parts.push(format!("next page cursor {cursor}"));
    }
    if parts.is_empty() {
        "The full result is in structuredContent.".to_owned()
    } else {
        format!(
            "{}. The full result is in structuredContent.",
            parts.join(", ")
        )
    }
}

#[derive(Debug)]
pub struct ToolFailure(Box<FailureParts>);

#[derive(Debug)]
pub struct FailureParts {
    pub error: Error,
    pub facts: AuditFacts,
}

impl ToolFailure {
    #[must_use]
    pub fn error(&self) -> &Error {
        &self.0.error
    }

    #[must_use]
    pub fn facts(&self) -> &AuditFacts {
        &self.0.facts
    }

    #[must_use]
    pub fn with_facts(mut self, facts: AuditFacts) -> Self {
        self.0.facts = facts;
        self
    }

    #[must_use]
    pub fn decision(&self) -> Decision {
        match self.0.error {
            Error::StatementRefused { .. }
            | Error::StatementMultiple { .. }
            | Error::StatementUnparsable { .. }
            | Error::RoleRefused { .. }
            | Error::ConfirmationRequired { .. }
            | Error::ScopeInsufficient { .. }
            | Error::AuditDegraded { .. } => Decision::Refused,
            _ => Decision::Allowed,
        }
    }

    #[must_use]
    pub fn rule(&self) -> Option<String> {
        match &self.0.error {
            Error::StatementRefused { rule, .. } => Some(rule.clone()),
            Error::StatementMultiple { count } => Some(format!("{count} statements in one call")),
            Error::StatementUnparsable { .. } => {
                Some("the parser could not read the statement".to_owned())
            }
            Error::RoleRefused { attribute, .. } => Some(format!("role has {attribute}")),
            Error::ConfirmationRequired { operation } => {
                Some(format!("{operation} needs confirmation"))
            }
            Error::ArgumentInvalid { argument, .. } => Some(format!("argument `{argument}`")),
            Error::ScopeInsufficient { scope } => Some(format!("token lacks scope {scope}")),
            Error::AuditDegraded { policy, .. } => Some(format!(
                "the audit log cannot be written (audit_on_failure = {policy})"
            )),
            _ => None,
        }
    }

    #[must_use]
    pub fn sqlstate(&self) -> Option<String> {
        match &self.0.error {
            Error::SqlFailed { sqlstate, .. } => sqlstate.clone(),
            _ => None,
        }
    }

    #[must_use]
    pub fn audit_outcome(&self) -> String {
        self.sqlstate()
            .unwrap_or_else(|| self.0.error.id().as_str().to_owned())
    }

    #[must_use]
    pub fn into_call_result(self) -> CallToolResult {
        let code = self.0.error.id().as_str();
        let message = crate::shape::sanitize(&self.0.error.to_string());
        let remedy = crate::shape::sanitize(&self.0.error.remedy());
        let mut structured = serde_json::Map::new();
        structured.insert(
            "code".to_owned(),
            serde_json::Value::String(code.to_owned()),
        );
        structured.insert(
            "message".to_owned(),
            serde_json::Value::String(message.clone()),
        );
        if let Some(rule) = self.rule() {
            structured.insert(
                "rule".to_owned(),
                serde_json::Value::String(crate::shape::sanitize(&rule)),
            );
        }
        if let Some(sqlstate) = self.sqlstate() {
            structured.insert("sqlstate".to_owned(), serde_json::Value::String(sqlstate));
        }
        structured.insert(
            "remedy".to_owned(),
            serde_json::Value::String(remedy.clone()),
        );
        let text = format!("error: {message}\ntry: {remedy}\ncode: {code}");
        let mut result = CallToolResult::structured_error(serde_json::Value::Object(structured));
        result.content = vec![ContentBlock::text(text)];
        result
    }
}

impl From<Error> for ToolFailure {
    fn from(error: Error) -> Self {
        Self(Box::new(FailureParts {
            error,
            facts: AuditFacts::default(),
        }))
    }
}

#[derive(Debug)]
pub enum Reply {
    Output(ToolOutput),
    InputRequired(InputRequiredResult),
}

impl From<ToolOutput> for Reply {
    fn from(output: ToolOutput) -> Self {
        Self::Output(output)
    }
}

pub type Outcome = Result<Reply, ToolFailure>;

pub type Handler = Arc<dyn Fn(Call) -> BoxFuture<'static, Outcome> + Send + Sync>;

#[derive(Clone)]
pub struct Route {
    pub spec: &'static ToolSpec,
    pub tool: Tool,
    pub handler: Handler,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("name", &self.spec.name)
            .finish_non_exhaustive()
    }
}

pub fn route<P, O, F>(
    spec: &'static ToolSpec,
    description: &'static str,
    handler: F,
) -> Result<Route, Error>
where
    P: DeserializeOwned + JsonSchema + Send + 'static,
    O: JsonSchema + 'static,
    F: Fn(Call, P) -> BoxFuture<'static, Outcome> + Send + Sync + 'static,
{
    let input_schema = schema_for_input::<P>().map_err(|reason| Error::ProtocolFailed {
        detail: format!("the input schema of {} is invalid: {reason}", spec.name),
    })?;
    let tool = Tool::new(spec.name, spec.description(description), input_schema)
        .with_title(spec.title)
        .with_raw_output_schema(schema_for_output::<O>())
        .with_annotations(spec.annotations());
    let handler = Arc::new(handler);
    let parsing_handler: Handler = Arc::new(move |call: Call| {
        let handler = Arc::clone(&handler);
        Box::pin(async move {
            let parsed: P = serde_json::from_value(serde_json::Value::Object(
                call.arguments.clone(),
            ))
            .map_err(|error| {
                ToolFailure::from(Error::ArgumentInvalid {
                    argument: "arguments".to_owned(),
                    detail: error.to_string(),
                })
            })?;
            handler(call, parsed).await
        })
    });
    Ok(Route {
        spec,
        tool,
        handler: parsing_handler,
    })
}

pub fn all_routes() -> Result<Vec<Route>, Error> {
    let mut routes = objects::routes()?;
    routes.extend(read::routes()?);
    routes.extend(health::routes()?);
    routes.extend(write::routes()?);
    routes.extend(transaction::routes()?);
    routes.extend(ddl::routes()?);
    routes.extend(roles::routes()?);
    routes.extend(maintenance::routes()?);
    routes.extend(host::routes()?);
    routes.extend(monitoring::routes()?);
    routes.extend(pooler::routes()?);
    Ok(routes)
}

#[must_use]
pub fn text_rows(
    columns: &[(&str, &str)],
    rows: Vec<Vec<Option<String>>>,
    cursor: Option<String>,
    estimate: Option<i64>,
) -> String {
    let caps = Caps {
        row_cap: rows.len().max(1),
        byte_cap: usize::MAX,
        cell_cap: crate::shape::CELL_CAP_BYTES,
    };
    let mut collector = crate::shape::Collector::new(
        columns
            .iter()
            .map(|(name, type_name)| crate::shape::Column {
                name: (*name).to_owned(),
                type_name: (*type_name).to_owned(),
            })
            .collect(),
    );
    for row in rows {
        collector.push(caps, row);
    }
    collector.finish(cursor, estimate).render_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(rows: usize, cursor: Option<&str>) -> ToolOutput {
        let text = "id\tname\n".to_owned() + &"1\tsome name\n".repeat(rows);
        ToolOutput {
            text,
            structured: serde_json::json!({
                "rows": [], "row_count": rows, "truncated": cursor.is_some(), "cursor": cursor
            }),
            facts: AuditFacts::default(),
        }
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|block| block.as_text())
            .map(|text| text.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn full_text_repeats_the_rows_and_summary_text_points_at_the_structured_result() {
        let full = output(100, Some("c1")).into_call_result(ResultText::Full);
        assert!(text_of(&full).starts_with("id\tname\n1\tsome name"));
        let summary = output(100, Some("c1")).into_call_result(ResultText::Summary);
        assert_eq!(
            text_of(&summary),
            "100 rows, more remain, next page cursor c1. The full result is in structuredContent."
        );
        assert_eq!(summary.structured_content.unwrap()["row_count"], 100);
        let one = output(60, None);
        let one = ToolOutput {
            structured: serde_json::json!({"row_count": 1, "truncated": false}),
            ..one
        }
        .into_call_result(ResultText::Summary);
        assert_eq!(
            text_of(&one),
            "1 row. The full result is in structuredContent."
        );
    }

    #[test]
    fn summary_text_keeps_a_short_text_as_it_is() {
        let short = output(2, None).into_call_result(ResultText::Summary);
        assert_eq!(text_of(&short), "id\tname\n1\tsome name\n1\tsome name\n");
    }

    #[test]
    fn a_failure_maps_onto_a_structured_error_with_the_stable_code() {
        let failure = ToolFailure::from(Error::StatementRefused {
            rule: "a write statement (InsertStmt) is not allowed".to_owned(),
            mode: "read-only".to_owned(),
        });
        assert_eq!(failure.decision(), Decision::Refused);
        let result = failure.into_call_result();
        assert_eq!(result.is_error, Some(true));
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["code"], "statement.refused");
        assert!(structured["rule"].as_str().unwrap().contains("InsertStmt"));
        assert!(structured["remedy"].as_str().unwrap().contains("mode"));
        assert!(structured.get("sqlstate").is_none());
    }

    #[test]
    fn a_sql_failure_carries_its_sqlstate_and_counts_as_allowed() {
        let failure = ToolFailure::from(Error::SqlFailed {
            sqlstate: Some("25006".to_owned()),
            message: "cannot execute nextval() in a read-only transaction".to_owned(),
        });
        assert_eq!(failure.decision(), Decision::Allowed);
        assert_eq!(failure.audit_outcome(), "25006");
        let structured = failure.into_call_result().structured_content.unwrap();
        assert_eq!(structured["sqlstate"], "25006");
        assert_eq!(structured["code"], "sql.failed");
    }

    #[test]
    fn every_route_matches_a_registry_entry_and_the_registry_is_fully_routed() {
        let routes = all_routes().unwrap();
        for route in &routes {
            assert_eq!(route.tool.name, route.spec.name);
            assert!(
                route.tool.description.as_ref().map_or(0, |d| d.len()) < 2_048,
                "{} has a description over 2 KiB",
                route.spec.name
            );
            let schema = serde_json::Value::Object((*route.tool.input_schema).clone());
            assert!(
                !schema.to_string().contains("\"null\""),
                "{} emits a null union: {schema}",
                route.spec.name
            );
            assert!(route.tool.output_schema.is_some(), "{}", route.spec.name);
        }
        for spec in crate::tool_specs::TOOLS {
            assert!(
                routes.iter().any(|route| route.spec.name == spec.name),
                "{} has no route",
                spec.name
            );
        }
        assert_eq!(routes.len(), crate::tool_specs::TOOLS.len());
    }
}
