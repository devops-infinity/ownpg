use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AuditFacts, Call, Outcome, Route, ToolOutput, route};
use crate::engine::HandleInfo;
use crate::error::Error;
use crate::groups;
use crate::render::validate_ident;

const DESCRIPTION: &str = "Open and control an explicit transaction. begin returns a handle; pass that handle as the transaction argument of the write and DDL tools so their statements run inside it, then commit or rollback. savepoint and rollback_to take a savepoint name. status reports a handle's state: open, expired, committed, rolled_back, or lost. One handle can be open at a time; it expires after the configured idle time and is rolled back, and a dropped connection marks it lost. Reads keep working over a second connection while a handle is open.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Begin,
    Commit,
    Rollback,
    Savepoint,
    RollbackTo,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransactionArgs {
    #[schemars(description = "What to do.")]
    pub operation: Operation,
    #[serde(default)]
    #[schemars(
        description = "The handle returned by begin. Required for every operation but begin."
    )]
    pub handle: String,
    #[serde(default)]
    #[schemars(description = "Savepoint name for savepoint and rollback_to.")]
    pub savepoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TransactionResult {
    pub operation: Operation,
    pub handle: HandleInfo,
}

pub fn transaction(call: Call, args: TransactionArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let engine = call.engine();
        let principal = call.principal.as_str();
        let needs_handle = args.operation != Operation::Begin;
        if needs_handle && args.handle.trim().is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "handle".to_owned(),
                detail: "the handle from begin is required".to_owned(),
            }
            .into());
        }
        let handle = match args.operation {
            Operation::Begin => engine.begin_transaction(principal).await?,
            Operation::Commit => engine.commit(&args.handle, principal).await?,
            Operation::Rollback => engine.rollback(&args.handle, principal).await?,
            Operation::Savepoint => {
                validate_ident("savepoint", &args.savepoint)?;
                engine
                    .savepoint(&args.handle, principal, &args.savepoint)
                    .await?
            }
            Operation::RollbackTo => {
                validate_ident("savepoint", &args.savepoint)?;
                engine
                    .rollback_to(&args.handle, principal, &args.savepoint)
                    .await?
            }
            Operation::Status => engine.transaction_status(&args.handle).await?,
        };
        let text = format!(
            "transaction {} is {}{}{}\n",
            handle.id,
            handle.state.as_str(),
            if handle.savepoints.is_empty() {
                String::new()
            } else {
                format!(", savepoints: {}", handle.savepoints.join(", "))
            },
            if handle.state == crate::engine::HandleState::Open {
                format!(", expires in {} s", handle.expires_in_seconds)
            } else {
                String::new()
            }
        );
        let facts = AuditFacts {
            operation: Some(format!("{:?}", args.operation).to_lowercase()),
            handle_id: Some(handle.id.clone()),
            ..AuditFacts::default()
        };
        let result = TransactionResult {
            operation: args.operation,
            handle,
        };
        Ok(ToolOutput::structured(&result, text)?
            .with_facts(facts)
            .into())
    })
}

pub fn routes() -> Result<Vec<Route>, Error> {
    Ok(vec![route::<TransactionArgs, TransactionResult, _>(
        &groups::PG_TRANSACTION,
        DESCRIPTION,
        transaction,
    )?])
}
