use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::confirm::{Verdict, ask};
use super::read::{classify_checked, facts_for};
use super::{AuditFacts, Call, Outcome, Route, ToolFailure, ToolOutput, route};
use crate::audit::Decision;
use crate::classify::{Classification, StatementClass};
use crate::error::Error;
use crate::groups;
use crate::render::{QualifiedName, expression, ident_list, quote_ident, quote_literal, verify};
use crate::shape::{ResultSet, UNTRUSTED_NOTICE};

const INSERT_DESCRIPTION: &str = "Insert one or more rows into a table of the scoped schema. Rows are JSON objects keyed by column name; values are converted to the column types by PostgreSQL. One statement inserts every row with the union of the columns the rows name: a column a row leaves out becomes NULL, and only a column no row names takes its default. on_conflict can ignore duplicates or update the listed columns. returning lists the columns to return (\"*\" for all). dry_run shows the statement without running it; transaction runs it inside an open handle.";

const UPDATE_DESCRIPTION: &str = "Update rows of a table in the scoped schema. set is a JSON object of column values; filter is a SQL boolean expression placed after WHERE. An update without a filter, or with a filter that is always true, touches every row and needs confirm: true (or the confirmation prompt when the client supports it). returning lists columns to return; on PostgreSQL 18, returning_old_new adds the old and new row images.";

const DELETE_DESCRIPTION: &str = "Delete rows from a table in the scoped schema. filter is a SQL boolean expression placed after WHERE. A delete without a filter, or with a filter that is always true, removes every row and needs confirm: true (or the confirmation prompt). returning lists columns to return from the deleted rows.";

const MERGE_DESCRIPTION: &str = "Upsert rows with MERGE (PostgreSQL 15 and later). rows are JSON objects; match_on names the columns that identify a row. Matched rows have update_columns set from the source (default: every column except match_on); unmatched rows are inserted when insert is true. returning (PostgreSQL 17 and later) lists columns to return.";

const RUN_WRITE_DESCRIPTION: &str = "Run one write statement written in SQL: INSERT, UPDATE, DELETE, MERGE, COPY ... FROM STDIN is refused here (use pg_copy), DO, or CALL. Exactly one statement per call. The statement is parsed and classified first: reads are refused (use pg_run_query), schema changes are refused (use the DDL tools), and destructive shapes (a DELETE or UPDATE without a narrowing WHERE) need confirm: true or the confirmation prompt. dry_run returns the classification without running anything.";

const COPY_DESCRIPTION: &str = "Move rows in bulk. direction in loads data into a table from the data argument through COPY FROM STDIN; direction out returns the rows of a table or a read query through COPY TO STDOUT, cut at the byte cap. Formats are text and csv; binary is refused. COPY never touches a file or a program on the database host.";

fn control_default_returning() -> Vec<String> {
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
    #[serde(default = "control_default_returning")]
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
    #[serde(default = "control_default_returning")]
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
    #[serde(default = "control_default_returning")]
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
    pub insert: bool,
    #[serde(default = "control_default_returning")]
    #[schemars(description = "PostgreSQL 17 and later: columns to return, or [\"*\"].")]
    pub returning: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Show the statement without running it.")]
    pub dry_run: bool,
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
    pub fingerprint: String,
    pub notice: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CopyOutResult {
    pub data: String,
    pub bytes: usize,
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

fn json_literal(value: &serde_json::Value) -> String {
    format!("{}::jsonb", quote_literal(&value.to_string()))
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
    let result = DryRun {
        dry_run: true,
        sql: sql.to_owned(),
        kind: classification.kind.clone(),
        class: classification.class.as_str().to_owned(),
        destructive: classification.destructive.clone(),
        fingerprint: classification.fingerprint.clone(),
        notice: UNTRUSTED_NOTICE,
    };
    let text = format!(
        "dry run: {} ({}){}\n{}\n",
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
    if dry_run {
        return dry_run_reply(sql, classification);
    }
    let decision = match call
        .context
        .gate
        .check(call, tool, classification, confirm)?
    {
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
    finish(result, facts)
}

fn finish(result: ResultSet, facts: AuditFacts) -> Outcome {
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
        let source_sql: Vec<String> = columns
            .iter()
            .map(|column| format!("source.{}", quote_ident(column)))
            .collect();
        let mut sql = format!(
            "INSERT INTO {} ({column_sql}) SELECT {} FROM jsonb_populate_recordset(NULL::{}, {}) AS source",
            table.sql(),
            source_sql.join(", "),
            table.sql(),
            json_literal(&serde_json::Value::Array(
                args.rows
                    .iter()
                    .cloned()
                    .map(serde_json::Value::Object)
                    .collect()
            ))
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
                let updates: Vec<String> = if args.update_columns.is_empty() {
                    columns
                        .iter()
                        .filter(|column| !args.conflict_columns.contains(column))
                        .cloned()
                        .collect()
                } else {
                    args.update_columns.clone()
                };
                ident_list("update_columns", &updates)?;
                let set: Vec<String> = updates
                    .iter()
                    .map(|column| {
                        let quoted = quote_ident(column);
                        format!("{quoted} = EXCLUDED.{quoted}")
                    })
                    .collect();
                sql.push_str(&format!(
                    " ON CONFLICT ({targets}) DO UPDATE SET {}",
                    set.join(", ")
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
        let picks: Vec<String> = columns
            .iter()
            .map(|column| format!("source.{}", quote_ident(column)))
            .collect();
        let mut sql = format!(
            "UPDATE {} SET ({column_sql}) = (SELECT {} FROM jsonb_populate_record(NULL::{}, {}) AS source)",
            table.sql(),
            picks.join(", "),
            table.sql(),
            json_literal(&serde_json::Value::Object(args.set.clone()))
        );
        if !args.filter.trim().is_empty() {
            sql.push_str(&format!(" WHERE {}", expression("filter", &args.filter)?));
        }
        if args.returning_old_new {
            if !call.engine().features().returning_old_new() {
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
        if !features.merge() {
            return Err(Error::ArgumentInvalid {
                argument: "operation".to_owned(),
                detail: "MERGE needs PostgreSQL 15 or later".to_owned(),
            }
            .into());
        }
        if !args.returning.is_empty() && !features.merge_returning() {
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
        let updates: Vec<String> = if args.update_columns.is_empty() {
            columns
                .iter()
                .filter(|column| !args.match_on.contains(column))
                .cloned()
                .collect()
        } else {
            ident_list("update_columns", &args.update_columns)?;
            args.update_columns.clone()
        };
        let joins: Vec<String> = args
            .match_on
            .iter()
            .map(|column| {
                let quoted = quote_ident(column);
                format!("target.{quoted} = source.{quoted}")
            })
            .collect();
        let mut sql = format!(
            "MERGE INTO {} AS target USING jsonb_populate_recordset(NULL::{}, {}) AS source ON {}",
            table.sql(),
            table.sql(),
            json_literal(&serde_json::Value::Array(
                args.rows
                    .iter()
                    .cloned()
                    .map(serde_json::Value::Object)
                    .collect()
            )),
            joins.join(" AND ")
        );
        if updates.is_empty() {
            sql.push_str(" WHEN MATCHED THEN DO NOTHING");
        } else {
            let set: Vec<String> = updates
                .iter()
                .map(|column| {
                    let quoted = quote_ident(column);
                    format!("{quoted} = source.{quoted}")
                })
                .collect();
            sql.push_str(&format!(" WHEN MATCHED THEN UPDATE SET {}", set.join(", ")));
        }
        if args.insert {
            let picks: Vec<String> = columns
                .iter()
                .map(|column| format!("source.{}", quote_ident(column)))
                .collect();
            sql.push_str(&format!(
                " WHEN NOT MATCHED THEN INSERT ({}) VALUES ({})",
                ident_list("rows", &columns)?,
                picks.join(", ")
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
            false,
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
        let classification = classify_checked(&call, &sql).await?;
        let mode = call.settings().mode.value;
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
                    bytes: data.len(),
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
        route::<InsertArgs, ResultSet, _>(&groups::PG_INSERT, INSERT_DESCRIPTION, insert)?,
        route::<UpdateArgs, ResultSet, _>(&groups::PG_UPDATE, UPDATE_DESCRIPTION, update)?,
        route::<DeleteArgs, ResultSet, _>(&groups::PG_DELETE, DELETE_DESCRIPTION, delete)?,
        route::<MergeArgs, ResultSet, _>(&groups::PG_MERGE, MERGE_DESCRIPTION, merge)?,
        route::<RunWriteArgs, ResultSet, _>(
            &groups::PG_RUN_WRITE,
            RUN_WRITE_DESCRIPTION,
            run_write,
        )?,
        route::<CopyArgs, CopyOutResult, _>(&groups::PG_COPY, COPY_DESCRIPTION, copy)?,
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
        let literal = json_literal(&serde_json::Value::Array(rows));
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
