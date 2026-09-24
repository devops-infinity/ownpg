use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::confirm::{Verdict, ask};
use super::read::{classify_checked, facts_for};
use super::{AuditFacts, Call, Outcome, Route, ToolFailure, ToolOutput, route};
use crate::audit::Decision;
use crate::classify::{self, Classification, SchemaScope, StatementClass};
use crate::config::Mode;
use crate::error::Error;
use crate::render::{QualifiedName, expression, ident_list, quote_ident, quote_literal, verify};
use crate::shape::{ResultSet, UNTRUSTED_NOTICE};
use crate::tool_specs;

const INSERT_DESCRIPTION: &str = "Insert one or more rows into a table of the scoped schema. Rows are JSON objects keyed by column name; values are converted to the column types by PostgreSQL. One statement inserts every row with the union of the columns the rows name: a column a row leaves out becomes NULL, and only a column no row names takes its default. Send a decimal with more than 15 significant digits as a JSON string so it arrives exactly; PostgreSQL converts the string to the column type. on_conflict can ignore duplicates or update the listed columns; with update, every row must name the same columns. returning lists the columns to return (\"*\" for all). dry_run shows the statement without running it; transaction runs it inside an open handle.";

const UPDATE_DESCRIPTION: &str = "Update rows of a table in the scoped schema. set is a JSON object of column values (send a decimal with more than 15 significant digits as a JSON string); filter is a SQL boolean expression placed after WHERE. An update without a filter, or with a filter that is always true, touches every row and needs confirm: true (or the confirmation prompt when the client supports it). returning lists columns to return; on PostgreSQL 18, returning_old_new adds the old and new row images.";

const DELETE_DESCRIPTION: &str = "Delete rows from a table in the scoped schema. filter is a SQL boolean expression placed after WHERE. A delete without a filter, or with a filter that is always true, removes every row and needs confirm: true (or the confirmation prompt). returning lists columns to return from the deleted rows.";

const MERGE_DESCRIPTION: &str = "Upsert rows with MERGE (PostgreSQL 15 and later). rows are JSON objects; match_on names the columns that identify a row. A matched row updates only the update_columns it names (default: every column the rows name except match_on), so a column a row leaves out keeps its stored value; unmatched rows are inserted when insert_unmatched is true. Two sessions that MERGE the same new key at the same time can hit a unique violation instead of one updating the other's row; for upserts that race, use pg_insert with on_conflict update, which PostgreSQL resolves safely. Send a decimal with more than 15 significant digits as a JSON string. returning (PostgreSQL 17 and later) lists columns to return. dry_run shows the statement without running it; confirm is accepted for the same reason it exists on pg_update and pg_delete, though match_on always builds a real join on named columns, so this tool never generates the unconditional MERGE the classifier would flag.";

const RUN_WRITE_DESCRIPTION: &str = "Run one write statement written in SQL: INSERT, UPDATE, DELETE, MERGE, COPY ... FROM STDIN is refused here (use pg_copy), DO, or CALL. Exactly one statement per call. The statement is parsed and classified first: reads are refused (use pg_run_query), schema changes are refused (use the DDL tools), and destructive shapes (a DELETE or UPDATE without a narrowing WHERE) need confirm: true or the confirmation prompt. dry_run returns the classification without running anything. A statement the parser cannot read is refused in every mode and never runs, because a statement without a class cannot be held to the access mode or the scoped schema; dry_run still reports it, with the kind unparsed and the class unknown.";

const COPY_DESCRIPTION: &str = "Move rows in bulk. direction in loads data into a table from the data argument through COPY FROM STDIN; direction out returns the rows of a table or a read query through COPY TO STDOUT, cut at the byte cap. Formats are text and csv; binary is refused. COPY never touches a file or a program on the database host.";

fn default_returning() -> Vec<String> {
    Vec::new()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    #[default]
    Error,
    DoNothing,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertArgs {
    #[schemars(description = "Table name, optionally schema-qualified.")]
    pub table: String,
    #[schemars(description = "Rows to insert, each a JSON object keyed by column name.")]
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    #[schemars(description = "error (default), do_nothing, or update.")]
    pub on_conflict: OnConflict,
    #[serde(default)]
    #[schemars(description = "Conflict target columns for do_nothing and update.")]
    pub conflict_columns: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "Columns to overwrite on conflict when on_conflict is update. Default: every inserted column except the conflict columns."
    )]
    pub update_columns: Vec<String>,
    #[serde(default = "default_returning")]
    #[schemars(
        description = "Columns to return, or [\"*\"] for every column. Empty returns nothing."
    )]
    pub returning: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Show the statement without running it.")]
    pub dry_run: bool,
    #[serde(default)]
    #[schemars(description = "Run inside this transaction handle.")]
    pub transaction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    #[schemars(description = "Table name, optionally schema-qualified.")]
    pub table: String,
    #[schemars(description = "Column values to set, as a JSON object.")]
    pub set: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    #[schemars(
        description = "SQL boolean expression placed after WHERE. Empty means every row and needs confirmation."
    )]
    pub filter: String,
    #[serde(default = "default_returning")]
    #[schemars(description = "Columns to return, or [\"*\"] for every column.")]
    pub returning: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "PostgreSQL 18 and later: also return the old and new row images as JSON."
    )]
    pub returning_old_new: bool,
    #[serde(default)]
    #[schemars(description = "Show the statement without running it.")]
    pub dry_run: bool,
    #[serde(default)]
    #[schemars(description = "Run a statement that touches every row.")]
    pub confirm: bool,
    #[serde(default)]
    #[schemars(description = "Run inside this transaction handle.")]
    pub transaction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteArgs {
    #[schemars(description = "Table name, optionally schema-qualified.")]
    pub table: String,
    #[serde(default)]
    #[schemars(
        description = "SQL boolean expression placed after WHERE. Empty means every row and needs confirmation."
    )]
    pub filter: String,
    #[serde(default = "default_returning")]
    #[schemars(description = "Columns to return from the deleted rows, or [\"*\"].")]
    pub returning: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Show the statement without running it.")]
    pub dry_run: bool,
    #[serde(default)]
    #[schemars(description = "Run a statement that removes every row.")]
    pub confirm: bool,
    #[serde(default)]
    #[schemars(description = "Run inside this transaction handle.")]
    pub transaction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeArgs {
    #[schemars(description = "Target table, optionally schema-qualified.")]
    pub table: String,
    #[schemars(description = "Source rows, each a JSON object keyed by column name.")]
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
    #[schemars(description = "Columns that identify a matching row.")]
    pub match_on: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "Columns to set on matched rows. Default: every source column except match_on."
    )]
    pub update_columns: Vec<String>,
    #[serde(default = "default_true")]
    #[schemars(description = "Insert rows that match nothing (default true).")]
    pub insert_unmatched: bool,
    #[serde(default = "default_returning")]
    #[schemars(description = "PostgreSQL 17 and later: columns to return, or [\"*\"].")]
    pub returning: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Show the statement without running it.")]
    pub dry_run: bool,
    #[serde(default)]
    #[schemars(description = "Run a MERGE the classifier marks destructive.")]
    pub confirm: bool,
    #[serde(default)]
    #[schemars(description = "Run inside this transaction handle.")]
    pub transaction: String,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunWriteArgs {
    #[schemars(description = "The single write statement to run.")]
    pub sql: String,
    #[serde(default)]
    #[schemars(description = "Classify the statement without running it.")]
    pub dry_run: bool,
    #[serde(default)]
    #[schemars(description = "Run a statement the classifier marks destructive.")]
    pub confirm: bool,
    #[serde(default)]
    #[schemars(description = "Run inside this transaction handle.")]
    pub transaction: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CopyDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CopyFormat {
    #[default]
    Text,
    Csv,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopyArgs {
    #[schemars(description = "in loads rows into a table; out returns rows.")]
    pub direction: CopyDirection,
    #[serde(default)]
    #[schemars(description = "Table name for in, and for out when query is empty.")]
    pub table: String,
    #[serde(default)]
    #[schemars(description = "Columns to load or return. Empty means every column.")]
    pub columns: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "For out: a read statement whose rows are returned instead of a table."
    )]
    pub query: String,
    #[serde(default)]
    #[schemars(description = "For in: the rows in text or csv form.")]
    pub data: String,
    #[serde(default)]
    #[schemars(description = "text (default) or csv.")]
    pub format: CopyFormat,
    #[serde(default)]
    #[schemars(description = "csv only: the first line is a header.")]
    pub header: bool,
    #[serde(default)]
    #[schemars(
        description = "Field delimiter; the default is a tab for text and a comma for csv."
    )]
    pub delimiter: String,
    #[serde(default)]
    #[schemars(
        description = "The string that stands for NULL; the default is \\N for text and an empty field for csv."
    )]
    pub null_string: String,
    #[serde(default)]
    #[schemars(description = "Show the statement without running it.")]
    pub dry_run: bool,
    #[serde(default)]
    #[schemars(description = "Run inside this transaction handle (in only).")]
    pub transaction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DryRun {
    pub dry_run: bool,
    pub sql: String,
    pub kind: String,
    pub class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cascades_to: Vec<String>,
    pub fingerprint: String,
    pub notice: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum StatementOutput {
    Rows(ResultSet),
    DryRun(DryRun),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum CopyOutput {
    Out(CopyOutResult),
    In(CopyInResult),
    DryRun(DryRun),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CopyOutResult {
    pub data: String,
    pub data_bytes: usize,
    pub truncated: bool,
    pub notice: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CopyInResult {
    pub rows_loaded: u64,
    pub sql: String,
}

pub fn returning_clause(columns: &[String]) -> Result<String, Error> {
    returning_prefixed(columns, "")
}

pub fn returning_prefixed(columns: &[String], prefix: &str) -> Result<String, Error> {
    if columns.is_empty() {
        return Ok(String::new());
    }
    if columns.len() == 1 && columns.first().is_some_and(|column| column == "*") {
        return Ok(format!(" RETURNING {prefix}*"));
    }
    ident_list("returning", columns)?;
    let listed: Vec<String> = columns
        .iter()
        .map(|column| format!("{prefix}{}", quote_ident(column)))
        .collect();
    Ok(format!(" RETURNING {}", listed.join(", ")))
}

fn exact_numbers(field: &str, value: &serde_json::Value) -> Result<(), Error> {
    match value {
        serde_json::Value::Number(number) if number.is_f64() => {
            let text = number.to_string();
            if crate::shape::significant_digits(&text) > crate::shape::SIGNIFICANT_DIGITS_KEPT {
                return Err(Error::ArgumentInvalid {
                    argument: field.to_owned(),
                    detail: format!(
                        "the number {text} has more significant digits than a JSON number keeps exactly, so its value may already be rounded; send exact decimal values as JSON strings, for example \"12345678901234567.89\""
                    ),
                });
            }
            Ok(())
        }
        serde_json::Value::Array(items) => {
            items.iter().try_for_each(|item| exact_numbers(field, item))
        }
        serde_json::Value::Object(map) => {
            map.values().try_for_each(|item| exact_numbers(field, item))
        }
        _ => Ok(()),
    }
}

fn json_literal(field: &str, value: &serde_json::Value) -> Result<String, Error> {
    exact_numbers(field, value)?;
    Ok(format!("{}::jsonb", quote_literal(&value.to_string())))
}

fn rows_value(rows: &[serde_json::Map<String, serde_json::Value>]) -> serde_json::Value {
    serde_json::Value::Array(
        rows.iter()
            .cloned()
            .map(serde_json::Value::Object)
            .collect(),
    )
}

fn update_targets(
    columns: &[String],
    keys: &[String],
    update_columns: &[String],
) -> Result<Vec<String>, Error> {
    if update_columns.is_empty() {
        return Ok(columns
            .iter()
            .filter(|column| !keys.contains(column))
            .cloned()
            .collect());
    }
    ident_list("update_columns", update_columns)?;
    if let Some(unset) = update_columns
        .iter()
        .find(|column| !columns.contains(column))
    {
        return Err(Error::ArgumentInvalid {
            argument: "update_columns".to_owned(),
            detail: format!(
                "`{unset}` is not set by any row, so there is no value to update it with"
            ),
        });
    }
    Ok(update_columns.to_vec())
}

fn unused_alias(columns: &[String], base: &str) -> String {
    let mut alias = base.to_owned();
    let mut counter = 0u32;
    while columns.iter().any(|column| column == &alias) {
        counter = counter.saturating_add(1);
        alias = format!("{base}_{counter}");
    }
    alias
}

fn column_union(rows: &[serde_json::Map<String, serde_json::Value>]) -> Result<Vec<String>, Error> {
    let mut columns: Vec<String> = Vec::new();
    for row in rows {
        for key in row.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    if columns.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "rows".to_owned(),
            detail: "at least one row with one column is required".to_owned(),
        });
    }
    ident_list("rows", &columns)?;
    Ok(columns)
}

pub fn dry_run_reply(sql: &str, classification: &Classification) -> Outcome {
    dry_run_listing(sql, classification, Vec::new())
}

fn dry_run_listing(
    sql: &str,
    classification: &Classification,
    cascades_to: Vec<String>,
) -> Outcome {
    let result = DryRun {
        dry_run: true,
        sql: sql.to_owned(),
        kind: classification.kind.clone(),
        class: classification.class.as_str().to_owned(),
        destructive: classification.destructive_reason.clone(),
        cascades_to,
        fingerprint: classification.fingerprint.clone(),
        notice: UNTRUSTED_NOTICE,
    };
    let cascade_text = if result.cascades_to.is_empty() {
        String::new()
    } else {
        format!("\nCASCADE also drops: {}", result.cascades_to.join(", "))
    };
    let text = format!(
        "dry run: {} ({}){}{cascade_text}\n{}\n",
        result.kind,
        result.class,
        result
            .destructive
            .as_ref()
            .map_or(String::new(), |rule| format!(", destructive: {rule}")),
        sql
    );
    let mut facts = facts_for(classification);
    facts.decision = Some(Decision::DryRun);
    Ok(ToolOutput::structured(&result, text)?
        .with_facts(facts)
        .into())
}

pub async fn execute(
    call: &Call,
    tool: &str,
    sql: &str,
    classification: &Classification,
    dry_run: bool,
    confirm: bool,
    transaction: &str,
) -> Outcome {
    let settings = call.settings();
    let pooled = call.engine().info().await.pooled;
    let scope = SchemaScope {
        schema: &settings.schema.value,
        require_qualified_names: pooled,
    };
    classify::authorize(classification, settings.mode.value, &scope)
        .map_err(|error| ToolFailure::from(error).with_facts(facts_for(classification)))?;
    let cascades_to = cascade_listing(call, classification)
        .await
        .map_err(|error| error.with_facts(facts_for(classification)))?;
    if dry_run {
        return dry_run_listing(sql, classification, cascades_to);
    }
    let mut checked = classification.clone();
    if !cascades_to.is_empty() {
        let also = format!("CASCADE also drops {}", cascades_to.join(", "));
        checked.destructive_reason = Some(match &classification.destructive_reason {
            Some(reason) => format!("{reason}; {also}"),
            None => also,
        });
    }
    let decision = match call.context.gate.check(call, tool, &checked, confirm)? {
        Verdict::Proceed(decision) => decision,
        Verdict::Ask(result) => return Ok(ask(*result)),
    };
    let mut facts = facts_for(classification);
    facts.decision = Some(decision);
    let handle = (!transaction.trim().is_empty()).then(|| transaction.trim());
    facts.handle_id = handle.map(str::to_owned);
    let result = call
        .engine()
        .run_write(
            sql,
            call.caps(0),
            &call.principal,
            handle,
            classification.runs_outside_transaction,
        )
        .await
        .map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
    rows_reply(result, facts)
}

async fn cascade_listing(
    call: &Call,
    classification: &Classification,
) -> Result<Vec<String>, ToolFailure> {
    if classification.cascade_targets.is_empty() {
        return Ok(Vec::new());
    }
    let dependents = super::cascade::dependents(call.engine(), classification).await?;
    let served = &call.settings().schema.value;
    let outside: Vec<&super::cascade::Dependent> = dependents
        .iter()
        .filter(|dependent| dependent.schema.as_deref() != Some(served.as_str()))
        .collect();
    if !outside.is_empty() {
        let named: Vec<String> = outside
            .iter()
            .take(super::cascade::LISTED_DEPENDENTS)
            .map(|dependent| dependent.label())
            .collect();
        return Err(Error::StatementRefused {
            rule: format!(
                "CASCADE would also drop objects outside the served schema `{served}`: {}; remove those dependencies first",
                named.join(", ")
            ),
            mode: call.settings().mode.value.to_string(),
        }
        .into());
    }
    Ok(super::cascade::listed(&dependents))
}

fn rows_reply(result: ResultSet, facts: AuditFacts) -> Outcome {
    let facts = facts.with_result(&result);
    let text = result.render_text();
    Ok(ToolOutput::structured(&result, text)?
        .with_facts(facts)
        .into())
}

pub fn insert(call: Call, args: InsertArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        let table = QualifiedName::parse("table", &args.table, &scoped)?;
        if args.rows.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "rows".to_owned(),
                detail: "at least one row is required".to_owned(),
            }
            .into());
        }
        let columns = column_union(&args.rows)?;
        let column_sql = ident_list("rows", &columns)?;
        let source_columns: Vec<String> = columns
            .iter()
            .map(|column| format!("source.{}", quote_ident(column)))
            .collect();
        let mut sql = format!(
            "INSERT INTO {} ({column_sql}) SELECT {} FROM jsonb_populate_recordset(NULL::{}, {}) AS source",
            table.sql(),
            source_columns.join(", "),
            table.sql(),
            json_literal("rows", &rows_value(&args.rows))?
        );
        match args.on_conflict {
            OnConflict::Error => {}
            OnConflict::DoNothing => {
                if args.conflict_columns.is_empty() {
                    sql.push_str(" ON CONFLICT DO NOTHING");
                } else {
                    sql.push_str(&format!(
                        " ON CONFLICT ({}) DO NOTHING",
                        ident_list("conflict_columns", &args.conflict_columns)?
                    ));
                }
            }
            OnConflict::Update => {
                let targets = ident_list("conflict_columns", &args.conflict_columns)?;
                if let Some(partial) = args.rows.iter().position(|row| row.len() != columns.len()) {
                    return Err(Error::ArgumentInvalid {
                        argument: "rows".to_owned(),
                        detail: format!(
                            "row {} names fewer columns than the others; with on_conflict update every row must name the same columns, because a missing column would overwrite the stored value with NULL. Use pg_merge to update only the columns each row names",
                            partial.saturating_add(1)
                        ),
                    }
                    .into());
                }
                let updates =
                    update_targets(&columns, &args.conflict_columns, &args.update_columns)?;
                let assignments: Vec<String> = updates
                    .iter()
                    .map(|column| {
                        let quoted = quote_ident(column);
                        format!("{quoted} = EXCLUDED.{quoted}")
                    })
                    .collect();
                sql.push_str(&format!(
                    " ON CONFLICT ({targets}) DO UPDATE SET {}",
                    assignments.join(", ")
                ));
            }
        }
        sql.push_str(&returning_clause(&args.returning)?);
        let classification = verify(&sql, &["InsertStmt"])?;
        execute(
            &call,
            "pg_insert",
            &sql,
            &classification,
            args.dry_run,
            false,
            &args.transaction,
        )
        .await
    })
}

pub fn update(call: Call, args: UpdateArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        let table = QualifiedName::parse("table", &args.table, &scoped)?;
        if args.set.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "set".to_owned(),
                detail: "at least one column is required".to_owned(),
            }
            .into());
        }
        let columns: Vec<String> = args.set.keys().cloned().collect();
        let column_sql = ident_list("set", &columns)?;
        let source_columns: Vec<String> = columns
            .iter()
            .map(|column| format!("source.{}", quote_ident(column)))
            .collect();
        let mut sql = format!(
            "UPDATE {} SET ({column_sql}) = (SELECT {} FROM jsonb_populate_record(NULL::{}, {}) AS source)",
            table.sql(),
            source_columns.join(", "),
            table.sql(),
            json_literal("set", &serde_json::Value::Object(args.set.clone()))?
        );
        if !args.filter.trim().is_empty() {
            sql.push_str(&format!(" WHERE {}", expression("filter", &args.filter)?));
        }
        if args.returning_old_new {
            if !call.engine().features().supports_returning_old_new() {
                return Err(Error::ArgumentInvalid {
                    argument: "returning_old_new".to_owned(),
                    detail: "RETURNING OLD and NEW needs PostgreSQL 18 or later".to_owned(),
                }
                .into());
            }
            let extra = if args.returning.is_empty() {
                String::new()
            } else if args.returning.first().is_some_and(|column| column == "*") {
                ", *".to_owned()
            } else {
                format!(", {}", ident_list("returning", &args.returning)?)
            };
            sql.push_str(&format!(
                " RETURNING WITH (OLD AS old_row, NEW AS new_row) to_jsonb(old_row) AS old_row, to_jsonb(new_row) AS new_row{extra}"
            ));
        } else {
            sql.push_str(&returning_clause(&args.returning)?);
        }
        let classification = verify(&sql, &["UpdateStmt"])?;
        execute(
            &call,
            "pg_update",
            &sql,
            &classification,
            args.dry_run,
            args.confirm,
            &args.transaction,
        )
        .await
    })
}

pub fn delete(call: Call, args: DeleteArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        let table = QualifiedName::parse("table", &args.table, &scoped)?;
        let mut sql = format!("DELETE FROM {}", table.sql());
        if !args.filter.trim().is_empty() {
            sql.push_str(&format!(" WHERE {}", expression("filter", &args.filter)?));
        }
        sql.push_str(&returning_clause(&args.returning)?);
        let classification = verify(&sql, &["DeleteStmt"])?;
        execute(
            &call,
            "pg_delete",
            &sql,
            &classification,
            args.dry_run,
            args.confirm,
            &args.transaction,
        )
        .await
    })
}

pub fn merge(call: Call, args: MergeArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let features = call.engine().features();
        if !features.supports_merge() {
            return Err(Error::ArgumentInvalid {
                argument: "operation".to_owned(),
                detail: "MERGE needs PostgreSQL 15 or later".to_owned(),
            }
            .into());
        }
        if !args.returning.is_empty() && !features.supports_merge_returning() {
            return Err(Error::ArgumentInvalid {
                argument: "returning".to_owned(),
                detail: "MERGE ... RETURNING needs PostgreSQL 17 or later".to_owned(),
            }
            .into());
        }
        let scoped = call.settings().schema.value.clone();
        let table = QualifiedName::parse("table", &args.table, &scoped)?;
        if args.rows.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "rows".to_owned(),
                detail: "at least one row is required".to_owned(),
            }
            .into());
        }
        let columns = column_union(&args.rows)?;
        ident_list("match_on", &args.match_on)?;
        for key in &args.match_on {
            if !columns.contains(key) {
                return Err(Error::ArgumentInvalid {
                    argument: "match_on".to_owned(),
                    detail: format!("`{key}` is not a column of the source rows"),
                }
                .into());
            }
        }
        let updates = update_targets(&columns, &args.match_on, &args.update_columns)?;
        let given = unused_alias(&columns, "ownpg_given");
        let selected: Vec<String> = columns
            .iter()
            .map(|column| format!("populated.{}", quote_ident(column)))
            .collect();
        let joins: Vec<String> = args
            .match_on
            .iter()
            .map(|column| {
                let quoted = quote_ident(column);
                format!("target.{quoted} = source.{quoted}")
            })
            .collect();
        let mut sql = format!(
            "MERGE INTO {} AS target USING (SELECT given.item AS {}, {} FROM jsonb_array_elements({}) AS given(item), jsonb_populate_record(NULL::{}, given.item) AS populated) AS source ON {}",
            table.sql(),
            quote_ident(&given),
            selected.join(", "),
            json_literal("rows", &rows_value(&args.rows))?,
            table.sql(),
            joins.join(" AND ")
        );
        if updates.is_empty() {
            sql.push_str(" WHEN MATCHED THEN DO NOTHING");
        } else {
            let assignments: Vec<String> = updates
                .iter()
                .map(|column| {
                    let quoted = quote_ident(column);
                    format!(
                        "{quoted} = CASE WHEN source.{} ? {} THEN source.{quoted} ELSE target.{quoted} END",
                        quote_ident(&given),
                        quote_literal(column)
                    )
                })
                .collect();
            sql.push_str(&format!(
                " WHEN MATCHED THEN UPDATE SET {}",
                assignments.join(", ")
            ));
        }
        if args.insert_unmatched {
            let source_columns: Vec<String> = columns
                .iter()
                .map(|column| format!("source.{}", quote_ident(column)))
                .collect();
            sql.push_str(&format!(
                " WHEN NOT MATCHED THEN INSERT ({}) VALUES ({})",
                ident_list("rows", &columns)?,
                source_columns.join(", ")
            ));
        }
        sql.push_str(&returning_prefixed(&args.returning, "target.")?);
        let classification = verify(&sql, &["MergeStmt"])?;
        execute(
            &call,
            "pg_merge",
            &sql,
            &classification,
            args.dry_run,
            args.confirm,
            &args.transaction,
        )
        .await
    })
}

pub fn run_write(call: Call, args: RunWriteArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let sql = args.sql.trim().to_owned();
        if sql.is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "sql".to_owned(),
                detail: "a statement is required".to_owned(),
            }
            .into());
        }
        let mode = call.settings().mode.value;
        let classification = match classify_checked(&call, &sql).await {
            Ok(classification) => classification,
            Err(failure)
                if mode == Mode::ReadWrite
                    && matches!(failure.error(), Error::StatementUnparsable { .. }) =>
            {
                return unparsed_reply(&sql, &args, failure);
            }
            Err(failure) => return Err(failure),
        };
        let refuse = |rule: String| {
            ToolFailure::from(Error::StatementRefused {
                rule,
                mode: mode.to_string(),
            })
            .with_facts(facts_for(&classification))
        };
        match classification.class {
            StatementClass::Write | StatementClass::Procedure => {}
            StatementClass::Read => {
                return Err(refuse(
                    "pg_run_write runs write statements; a read belongs to pg_run_query".to_owned(),
                ));
            }
            StatementClass::Ddl | StatementClass::Roles | StatementClass::Maintenance => {
                return Err(refuse(format!(
                    "pg_run_write runs data statements; a {} statement ({}) belongs to the {} tools",
                    classification.class, classification.kind, classification.class
                )));
            }
        }
        if classification.kind == "CopyStmt" {
            return Err(refuse(
                "COPY runs through pg_copy, which streams the data".to_owned(),
            ));
        }
        execute(
            &call,
            "pg_run_write",
            &sql,
            &classification,
            args.dry_run,
            args.confirm,
            &args.transaction,
        )
        .await
    })
}

fn unparsed_reply(sql: &str, args: &RunWriteArgs, failure: ToolFailure) -> Outcome {
    let fingerprint: String = crate::audit::sha256_hex(sql.as_bytes())
        .chars()
        .take(16)
        .collect();
    let mut facts = AuditFacts {
        operation: Some("unparsed".to_owned()),
        statement_hash: Some(fingerprint.clone()),
        ..AuditFacts::default()
    };
    if args.dry_run {
        facts.decision = Some(Decision::DryRun);
        let result = DryRun {
            dry_run: true,
            sql: sql.to_owned(),
            kind: "unparsed".to_owned(),
            class: "unknown".to_owned(),
            destructive: Some(
                "the parser could not read this statement, so its class and effect are unknown"
                    .to_owned(),
            ),
            cascades_to: Vec::new(),
            fingerprint,
            notice: UNTRUSTED_NOTICE,
        };
        let text = format!(
            "dry run: unparsed statement; it is refused rather than run, because a statement without a class cannot be held to the access mode or the scoped schema
{sql}
"
        );
        return Ok(ToolOutput::structured(&result, text)?
            .with_facts(facts)
            .into());
    }
    Err(failure.with_facts(facts))
}

fn copy_options(args: &CopyArgs) -> Result<String, Error> {
    let mut options = vec![match args.format {
        CopyFormat::Text => "FORMAT text".to_owned(),
        CopyFormat::Csv => "FORMAT csv".to_owned(),
    }];
    if args.header {
        if args.format != CopyFormat::Csv {
            return Err(Error::ArgumentInvalid {
                argument: "header".to_owned(),
                detail: "a header line needs the csv format".to_owned(),
            });
        }
        options.push("HEADER true".to_owned());
    }
    if !args.delimiter.is_empty() {
        if args.delimiter.chars().count() != 1 {
            return Err(Error::ArgumentInvalid {
                argument: "delimiter".to_owned(),
                detail: "the delimiter is one character".to_owned(),
            });
        }
        options.push(format!("DELIMITER {}", quote_literal(&args.delimiter)));
    }
    if !args.null_string.is_empty() {
        options.push(format!("NULL {}", quote_literal(&args.null_string)));
    }
    Ok(options.join(", "))
}

pub fn copy(call: Call, args: CopyArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let scoped = call.settings().schema.value.clone();
        let options = copy_options(&args)?;
        let column_sql = if args.columns.is_empty() {
            String::new()
        } else {
            format!(" ({})", ident_list("columns", &args.columns)?)
        };
        match args.direction {
            CopyDirection::In => {
                if args.table.trim().is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "table".to_owned(),
                        detail: "a table is required to load into".to_owned(),
                    }
                    .into());
                }
                let table = QualifiedName::parse("table", &args.table, &scoped)?;
                let sql = format!(
                    "COPY {}{column_sql} FROM STDIN WITH ({options})",
                    table.sql()
                );
                let classification = verify(&sql, &["CopyStmt"])?;
                if args.dry_run {
                    return dry_run_reply(&sql, &classification);
                }
                if args.data.is_empty() {
                    return Err(Error::ArgumentInvalid {
                        argument: "data".to_owned(),
                        detail: "no rows were given".to_owned(),
                    }
                    .into());
                }
                let mut facts = facts_for(&classification);
                let handle = (!args.transaction.trim().is_empty()).then(|| args.transaction.trim());
                facts.handle_id = handle.map(str::to_owned);
                let loaded = call
                    .engine()
                    .copy_in(&sql, args.data.as_bytes(), &call.principal, handle)
                    .await
                    .map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
                facts.row_count = Some(loaded);
                let result = CopyInResult {
                    rows_loaded: loaded,
                    sql,
                };
                let text = format!("loaded {} rows into {}\n", loaded, table.display());
                Ok(ToolOutput::structured(&result, text)?
                    .with_facts(facts)
                    .into())
            }
            CopyDirection::Out => {
                let source = if args.query.trim().is_empty() {
                    if args.table.trim().is_empty() {
                        return Err(Error::ArgumentInvalid {
                            argument: "table".to_owned(),
                            detail: "a table or a query is required".to_owned(),
                        }
                        .into());
                    }
                    let table = QualifiedName::parse("table", &args.table, &scoped)?;
                    format!("{}{column_sql}", table.sql())
                } else {
                    let inner = classify_checked(&call, args.query.trim()).await?;
                    if inner.class != StatementClass::Read || inner.kind != "SelectStmt" {
                        return Err(Error::StatementRefused {
                            rule: "COPY out takes a SELECT".to_owned(),
                            mode: call.settings().mode.value.to_string(),
                        }
                        .into());
                    }
                    format!("({})", args.query.trim())
                };
                let sql = format!("COPY {source} TO STDOUT WITH ({options})");
                let classification = verify(&sql, &["CopyStmt"])?;
                if args.dry_run {
                    return dry_run_reply(&sql, &classification);
                }
                let facts = facts_for(&classification);
                let byte_cap = call.caps(0).byte_cap;
                let (bytes, truncated) = call
                    .engine()
                    .copy_out(&sql, byte_cap)
                    .await
                    .map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
                let data = crate::shape::sanitize(&String::from_utf8_lossy(&bytes));
                let result = CopyOutResult {
                    data_bytes: data.len(),
                    truncated,
                    data,
                    notice: UNTRUSTED_NOTICE,
                };
                let text = format!(
                    "{}\n{}{}",
                    UNTRUSTED_NOTICE,
                    result.data,
                    if truncated {
                        "\n(cut at the byte cap)\n"
                    } else {
                        ""
                    }
                );
                Ok(ToolOutput::structured(&result, text)?
                    .with_facts(AuditFacts { truncated, ..facts })
                    .into())
            }
        }
    })
}

pub fn routes() -> Result<Vec<Route>, Error> {
    Ok(vec![
        route::<InsertArgs, StatementOutput, _>(
            &tool_specs::PG_INSERT,
            INSERT_DESCRIPTION,
            insert,
        )?,
        route::<UpdateArgs, StatementOutput, _>(
            &tool_specs::PG_UPDATE,
            UPDATE_DESCRIPTION,
            update,
        )?,
        route::<DeleteArgs, StatementOutput, _>(
            &tool_specs::PG_DELETE,
            DELETE_DESCRIPTION,
            delete,
        )?,
        route::<MergeArgs, StatementOutput, _>(&tool_specs::PG_MERGE, MERGE_DESCRIPTION, merge)?,
        route::<RunWriteArgs, StatementOutput, _>(
            &tool_specs::PG_RUN_WRITE,
            RUN_WRITE_DESCRIPTION,
            run_write,
        )?,
        route::<CopyArgs, CopyOutput, _>(&tool_specs::PG_COPY, COPY_DESCRIPTION, copy)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returning_renders_star_or_a_quoted_list() {
        assert_eq!(returning_clause(&[]).unwrap(), "");
        assert_eq!(returning_clause(&["*".to_owned()]).unwrap(), " RETURNING *");
        assert_eq!(
            returning_clause(&["id".to_owned(), "Name".to_owned()]).unwrap(),
            " RETURNING \"id\", \"Name\""
        );
        assert!(returning_clause(&[String::new()]).is_err());
    }

    #[test]
    fn json_rows_become_a_quoted_jsonb_literal() {
        let rows = vec![serde_json::json!({"id": 1, "note": "it's"})];
        let literal = json_literal("rows", &serde_json::Value::Array(rows)).unwrap();
        assert_eq!(literal, "'[{\"id\":1,\"note\":\"it''s\"}]'::jsonb");
        let columns = column_union(&[
            serde_json::json!({"b": 1, "a": 2})
                .as_object()
                .unwrap()
                .clone(),
            serde_json::json!({"c": 3}).as_object().unwrap().clone(),
        ])
        .unwrap();
        assert_eq!(columns, ["a", "b", "c"]);
        assert!(column_union(&[serde_json::Map::new()]).is_err());
    }

    #[test]
    fn an_unparsed_statement_keeps_only_its_hash_in_the_audit_facts() {
        let sql = "ALTER ROLE app PASSWORD 'hunter2' VALID (";
        let unparsable = || {
            ToolFailure::from(Error::StatementUnparsable {
                reason: "syntax error".to_owned(),
            })
        };
        let refused_args = RunWriteArgs {
            sql: sql.to_owned(),
            dry_run: false,
            confirm: false,
            transaction: String::new(),
        };
        let Err(refused) = unparsed_reply(sql, &refused_args, unparsable()) else {
            panic!("an unparsed statement outside a dry run must be refused");
        };
        assert!(refused.facts().statement.is_none());
        assert_eq!(
            refused.facts().statement_hash.as_deref().map(str::len),
            Some(16)
        );
        let dry_args = RunWriteArgs {
            dry_run: true,
            ..refused_args
        };
        let Ok(crate::tools::Reply::Output(output)) = unparsed_reply(sql, &dry_args, unparsable())
        else {
            panic!("a dry run of an unparsed statement returns a report");
        };
        assert!(output.facts.statement.is_none());
        assert!(!format!("{:?}", output.facts).contains("hunter2"));
    }

    #[test]
    fn numbers_that_may_have_lost_precision_are_refused() {
        let exact = serde_json::json!([{"a": 0.1, "b": 123456789012.345, "c": 1e20, "d": 9007199254740993_u64, "e": -0.000123}]);
        assert!(json_literal("rows", &exact).is_ok());
        let rounded: serde_json::Value =
            serde_json::from_str(r#"[{"amount": 12345678901234567.89}]"#).unwrap();
        let error = json_literal("rows", &rounded).unwrap_err();
        assert!(error.to_string().contains("JSON strings"), "{error}");
        let nested: serde_json::Value =
            serde_json::from_str(r#"{"doc": {"ratio": 0.12345678901234567}}"#).unwrap();
        assert!(json_literal("set", &nested).is_err());
        let as_text = serde_json::json!([{"amount": "12345678901234567.89"}]);
        assert!(json_literal("rows", &as_text).is_ok());
    }

    #[test]
    fn update_columns_must_be_set_by_a_row() {
        let columns = ["id".to_owned(), "name".to_owned()];
        assert_eq!(
            update_targets(&columns, &["id".to_owned()], &[]).unwrap(),
            ["name"]
        );
        let error =
            update_targets(&columns, &["id".to_owned()], &["email".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("email"), "{error}");
        assert_eq!(unused_alias(&columns, "ownpg_given"), "ownpg_given");
        assert_eq!(
            unused_alias(&["ownpg_given".to_owned()], "ownpg_given"),
            "ownpg_given_1"
        );
    }

    #[test]
    fn copy_options_follow_the_format() {
        let args: CopyArgs = serde_json::from_value(serde_json::json!({
            "direction": "in", "table": "t", "format": "csv", "header": true, "delimiter": ";"
        }))
        .unwrap();
        assert_eq!(
            copy_options(&args).unwrap(),
            "FORMAT csv, HEADER true, DELIMITER ';'"
        );
        let bad: CopyArgs = serde_json::from_value(serde_json::json!({
            "direction": "in", "table": "t", "header": true
        }))
        .unwrap();
        assert!(copy_options(&bad).is_err());
        let rejected = serde_json::from_value::<CopyArgs>(serde_json::json!({
            "direction": "in", "table": "t", "format": "binary"
        }));
        assert!(rejected.is_err());
    }
}
