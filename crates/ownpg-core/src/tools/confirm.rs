use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use rmcp::model::{
    BooleanSchema, ElicitRequest, ElicitRequestParams, ElicitationAction, ElicitationSchema,
    InputRequest, InputRequiredResult, PrimitiveSchemaDefinition, RequestStateCodec, SealOptions,
};
use serde::{Deserialize, Serialize};

use super::{AuditFacts, Call, Reply, ToolFailure};
use crate::audit::Decision;
use crate::classify::Classification;
use crate::error::Error;

pub const CONFIRMATION_TTL: Duration = Duration::from_secs(300);
pub const REQUEST_KEY: &str = "confirm";
pub const PENDING_FORMAT: u32 = 1;
const TARGET_DIGEST_HEX_CHARS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pending {
    pub v: u32,
    pub tool: String,
    pub sql_sha256: String,
    pub principal: String,
    pub target: String,
    pub issued_at: u64,
    pub nonce: u64,
}

#[derive(Debug)]
pub struct Gate {
    codec: RequestStateCodec,
    consumed: std::sync::Mutex<VecDeque<(u64, Instant)>>,
    started_at: u64,
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[must_use]
pub fn target_digest(settings: &crate::config::Settings) -> String {
    let connection = &settings.connection;
    let identity = format!(
        "{}\n{}\n{}\n{}",
        connection
            .host
            .as_ref()
            .map_or("", |host| host.value.as_str()),
        connection.port.value,
        settings.database.value,
        settings.schema.value
    );
    crate::audit::sha256_hex(identity.as_bytes())
        .chars()
        .take(TARGET_DIGEST_HEX_CHARS)
        .collect()
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum Verdict {
    Proceed(Decision),
    Ask(Box<InputRequiredResult>),
}

impl Gate {
    #[must_use]
    pub fn new() -> Self {
        let key: [u8; 32] = rand::random();
        Self::with_key(&key)
    }

    #[must_use]
    pub fn with_key(key: &[u8]) -> Self {
        Self {
            codec: RequestStateCodec::new_unchecked(key.to_vec()),
            consumed: std::sync::Mutex::new(VecDeque::new()),
            started_at: unix_seconds(),
        }
    }

    fn matches(
        &self,
        pending: &Pending,
        tool: &str,
        sql_sha256: &str,
        principal: &str,
        target: &str,
    ) -> Result<(), Error> {
        let refusal = |detail: &str| Error::ConfirmationRequired {
            operation: format!("{detail}; confirm again"),
        };
        if pending.v != PENDING_FORMAT {
            return Err(refusal(
                "the confirmation was issued by another version of OwnPG",
            ));
        }
        if pending.tool != tool
            || pending.sql_sha256 != sql_sha256
            || pending.principal != principal
        {
            return Err(Error::ConfirmationRequired {
                operation: "the confirmation belongs to a different statement or caller".to_owned(),
            });
        }
        if pending.target != target {
            return Err(refusal(
                "the confirmation was issued for another database or schema",
            ));
        }
        if pending.issued_at < self.started_at {
            return Err(refusal(
                "the confirmation was issued before this server started",
            ));
        }
        Ok(())
    }

    fn consume(&self, nonce: u64) -> Result<(), Error> {
        let mut consumed = self.consumed.lock().map_err(|_| Error::ProtocolFailed {
            detail: "the confirmation ledger is poisoned".to_owned(),
        })?;
        let now = Instant::now();
        while consumed
            .front()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) > CONFIRMATION_TTL)
        {
            consumed.pop_front();
        }
        if consumed.iter().any(|(used, _)| *used == nonce) {
            return Err(Error::ConfirmationRequired {
                operation: "this confirmation was already used once; confirm again".to_owned(),
            });
        }
        consumed.push_back((nonce, now));
        Ok(())
    }

    pub fn from_key_file(path: &std::path::Path) -> Result<Self, Error> {
        let file = crate::config::profile::open_private(path)?;
        let bytes = crate::config::profile::read_capped_bytes(file, path)?;
        if bytes.len() < 32 {
            return Err(Error::ConfigInvalid {
                setting: "http.state_key_file".to_owned(),
                value: path.display().to_string(),
                detail: "the key file needs at least 32 bytes".to_owned(),
            });
        }
        Ok(Self::with_key(&bytes))
    }

    pub fn seal(&self, pending: &Pending) -> Result<String, Error> {
        let options = SealOptions::new().ttl(CONFIRMATION_TTL);
        self.codec
            .seal_json_with(pending, &options)
            .map_err(|error| Error::ProtocolFailed {
                detail: format!("the confirmation state could not be sealed: {error}"),
            })
    }

    pub fn open(&self, sealed: &str) -> Result<Pending, Error> {
        self.codec
            .open_json(sealed)
            .map_err(|error| Error::ConfirmationRequired {
                operation: format!("the confirmation state is not valid ({error}); confirm again"),
            })
    }

    fn ask(
        &self,
        call: &Call,
        tool: &str,
        classification: &Classification,
        rule: &str,
    ) -> Result<Verdict, ToolFailure> {
        let sealed = self.seal(&Pending {
            v: PENDING_FORMAT,
            tool: tool.to_owned(),
            sql_sha256: classification.sql_sha256.clone(),
            principal: call.principal.clone(),
            target: target_digest(call.settings()),
            issued_at: unix_seconds(),
            nonce: rand::random(),
        })?;
        let message = format!(
            "This {} statement is destructive: {rule}. Statement: {}. Run it?",
            classification.kind,
            short_statement(&classification.normalized)
        );
        let schema = ElicitationSchema::builder()
            .required_property(
                "proceed",
                PrimitiveSchemaDefinition::Boolean(
                    BooleanSchema::new()
                        .title("Run the statement")
                        .description("true runs it now; false leaves the database untouched"),
                ),
            )
            .build()
            .map_err(|reason| Error::ProtocolFailed {
                detail: format!("the confirmation form is invalid: {reason}"),
            })?;
        let request = ElicitRequest::new(ElicitRequestParams::FormElicitationParams {
            meta: None,
            message,
            requested_schema: schema,
        });
        let mut requests = BTreeMap::new();
        requests.insert(REQUEST_KEY.to_owned(), InputRequest::Elicitation(request));
        Ok(Verdict::Ask(Box::new(InputRequiredResult::new(
            Some(requests),
            Some(sealed),
        ))))
    }

    pub fn check(
        &self,
        call: &Call,
        tool: &str,
        classification: &Classification,
        confirm: bool,
    ) -> Result<Verdict, ToolFailure> {
        let Some(rule) = classification.destructive_reason.as_deref() else {
            return Ok(Verdict::Proceed(Decision::Allowed));
        };
        if confirm {
            return Ok(Verdict::Proceed(Decision::ConfirmedArgument));
        }
        if let Some(sealed) = call.request_state.as_deref() {
            let pending = self.open(sealed)?;
            self.matches(
                &pending,
                tool,
                &classification.sql_sha256,
                &call.principal,
                &target_digest(call.settings()),
            )?;
            return match answer(call.input_responses.as_ref()) {
                Answer::Accepted => {
                    self.consume(pending.nonce)?;
                    Ok(Verdict::Proceed(Decision::ConfirmedElicitation))
                }
                Answer::Declined => Err(ToolFailure::from(Error::StatementRefused {
                    rule: format!("declined at the confirmation prompt: {rule}"),
                    mode: call.settings().mode.value.to_string(),
                })
                .with_facts(AuditFacts {
                    decision: Some(Decision::Refused),
                    ..crate::tools::read::facts_for(classification)
                })),
                Answer::Missing => self.ask(call, tool, classification, rule),
            };
        }
        if call.can_elicit {
            return self.ask(call, tool, classification, rule);
        }
        Err(Error::ConfirmationRequired {
            operation: format!(
                "{rule}; pass confirm: true to run it, or dry_run: true to see the statement first"
            ),
        }
        .into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Accepted,
    Declined,
    Missing,
}

fn answer(responses: Option<&rmcp::model::InputResponses>) -> Answer {
    let Some(value) = responses.and_then(|map| map.get(REQUEST_KEY)) else {
        return Answer::Missing;
    };
    let action = value
        .get("action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let accepted =
        serde_json::from_value::<ElicitationAction>(serde_json::Value::String(action.to_owned()))
            .is_ok_and(|parsed| parsed == ElicitationAction::Accept);
    if !accepted {
        return Answer::Declined;
    }
    let proceed = value
        .pointer("/content/proceed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if proceed {
        Answer::Accepted
    } else {
        Answer::Declined
    }
}

fn short_statement(statement: &str) -> String {
    crate::audit::short_statement(statement).unwrap_or_else(|| {
        crate::shape::cut_graphemes(statement, crate::audit::SHORT_STATEMENT_CAP)
    })
}

#[must_use]
pub fn ask(result: InputRequiredResult) -> Reply {
    Reply::InputRequired(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_confirmation_round_trips_and_a_foreign_one_is_refused() {
        let gate = Gate::new();
        let pending = pending();
        let sealed = gate.seal(&pending).unwrap();
        assert_eq!(gate.open(&sealed).unwrap(), pending);
        let other = Gate::new();
        assert!(other.open(&sealed).is_err());
        assert!(gate.open("not-a-token").is_err());
    }

    fn pending() -> Pending {
        Pending {
            v: PENDING_FORMAT,
            tool: "pg_delete".to_owned(),
            sql_sha256: "abc".to_owned(),
            principal: "tester".to_owned(),
            target: "t1".to_owned(),
            issued_at: unix_seconds(),
            nonce: 7,
        }
    }

    #[test]
    fn a_confirmation_only_fits_its_statement_caller_target_and_server_run() {
        let gate = Gate::new();
        let fresh = pending();
        gate.matches(&fresh, "pg_delete", "abc", "tester", "t1")
            .unwrap();
        for (tool, sql, principal, target) in [
            ("pg_update", "abc", "tester", "t1"),
            ("pg_delete", "abd", "tester", "t1"),
            ("pg_delete", "abc", "mallory", "t1"),
            ("pg_delete", "abc", "tester", "t2"),
        ] {
            let refused = gate
                .matches(&fresh, tool, sql, principal, target)
                .unwrap_err();
            assert_eq!(refused.id().as_str(), "confirmation.required");
        }
        let earlier = Pending {
            issued_at: gate.started_at - 1,
            ..pending()
        };
        let refused = gate
            .matches(&earlier, "pg_delete", "abc", "tester", "t1")
            .unwrap_err();
        assert!(
            refused.remedy().contains("before this server started"),
            "{}",
            refused.remedy()
        );
        let older_format = Pending { v: 0, ..pending() };
        assert!(
            gate.matches(&older_format, "pg_delete", "abc", "tester", "t1")
                .is_err()
        );
    }

    #[test]
    fn a_confirmation_nonce_is_accepted_once() {
        let gate = Gate::new();
        gate.consume(42).unwrap();
        let again = gate.consume(42).unwrap_err();
        assert_eq!(again.id().as_str(), "confirmation.required");
        gate.consume(43).unwrap();
    }

    #[test]
    fn answers_are_read_from_the_elicitation_result() {
        let mut responses = rmcp::model::InputResponses::new();
        assert_eq!(answer(Some(&responses)), Answer::Missing);
        responses.insert(
            REQUEST_KEY.to_owned(),
            serde_json::json!({"action": "accept", "content": {"proceed": true}}),
        );
        assert_eq!(answer(Some(&responses)), Answer::Accepted);
        responses.insert(
            REQUEST_KEY.to_owned(),
            serde_json::json!({"action": "accept", "content": {"proceed": false}}),
        );
        assert_eq!(answer(Some(&responses)), Answer::Declined);
        responses.insert(
            REQUEST_KEY.to_owned(),
            serde_json::json!({"action": "cancel"}),
        );
        assert_eq!(answer(Some(&responses)), Answer::Declined);
    }
}
