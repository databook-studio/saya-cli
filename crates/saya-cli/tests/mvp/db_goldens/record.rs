//! The golden record: what a scenario saw, how it is compared, and the one
//! opt-in path that regenerates it.
//!
//! A record is the *behaviour* the next database-plugin refactor must keep:
//! what each provider request advertised (tool names in order, normalised
//! body size, message count), every tool call the model made with its
//! outcome class, and the user-visible state of every settled turn. It holds
//! no raw row values, ids, paths, or timestamps.
//!
//! Regenerate deliberately with `SAYA_UPDATE_GOLDENS=1 cargo nextest run -p
//! saya-cli --locked --test mvp -E 'test(db_goldens)'`, then review the diff
//! like code: a changed golden is a behaviour change, not a rubber stamp.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::harness::ServedCall;
use super::normalise::Normaliser;

/// One scenario's committed golden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Golden {
    pub(crate) scenario: String,
    pub(crate) requests: Vec<RequestRecord>,
    pub(crate) tool_calls: Vec<ToolCallRecord>,
    /// One entry per settled turn, in the order the user saw them.
    pub(crate) states: Vec<StateRecord>,
}

/// What one provider request advertised and carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RequestRecord {
    /// The advertised tool names, in wire order.
    pub(crate) tools: Vec<String>,
    /// The body's byte length after normalisation.
    pub(crate) body_bytes: usize,
    pub(crate) messages: usize,
}

/// One tool call the model made and what the executor answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolCallRecord {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sql: Option<String>,
    /// `ok`, `refused-by-safety`, or `error`.
    pub(crate) outcome: String,
}

/// The user-visible outcome of one settled turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StateRecord {
    /// `answer`, `clarification`, or `budget-stop`.
    pub(crate) kind: String,
    pub(crate) text: String,
}

impl Golden {
    /// Builds a record from the mock's request bodies, the tool calls it
    /// served, and the scenario's settled states.
    pub(crate) fn build(
        scenario: &str,
        bodies: &[String],
        served: &[ServedCall],
        normaliser: &Normaliser,
        states: Vec<StateRecord>,
    ) -> Self {
        let mut requests = Vec::with_capacity(bodies.len());
        let mut results: HashMap<String, String> = HashMap::new();

        for body in bodies {
            let normalised = normaliser.normalise(body);
            let value: serde_json::Value = serde_json::from_str(&normalised)
                .unwrap_or_else(|error| panic!("request body is JSON: {error}; {normalised}"));
            let messages = value["messages"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            requests.push(RequestRecord {
                tools: advertised_tools(&value),
                body_bytes: normalised.len(),
                messages: messages.len(),
            });
            for message in messages {
                record_tool_result(message, &mut results);
            }
        }

        let clarified = states.iter().any(|state| state.kind == "clarification");
        let tool_calls = served
            .iter()
            .map(|call| ToolCallRecord {
                name: call.name.clone(),
                sql: call.arguments["sql"].as_str().map(str::to_owned),
                outcome: match results.get(&call.id) {
                    Some(content) => classify_result(content).to_owned(),
                    // A clarification ends its turn, so its call is not
                    // replayed in the next request. It succeeded exactly when
                    // the user saw the question.
                    None if call.name == "request_clarification" && clarified => "ok".to_owned(),
                    // Any other call with no result reached no model turn:
                    // the turn stopped first.
                    None => "error".to_owned(),
                },
            })
            .collect();

        Golden {
            scenario: scenario.to_owned(),
            requests,
            tool_calls,
            states,
        }
    }
}

fn advertised_tools(value: &serde_json::Value) -> Vec<String> {
    value["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn record_tool_result(message: &serde_json::Value, results: &mut HashMap<String, String>) {
    if message["role"] != "tool" {
        return;
    }
    if let (Some(id), Some(content)) = (
        message["tool_call_id"].as_str(),
        message["content"].as_str(),
    ) {
        results.insert(id.to_owned(), content.to_owned());
    }
}

/// Classifies a tool result the model received. A safety refusal is named by
/// the one policy phrase the safety layer emits; any other error is `error`.
pub(crate) fn classify_result(content: &str) -> &'static str {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            return classify_error(error);
        }
        return "ok";
    }
    classify_error(content)
}

fn classify_error(text: &str) -> &'static str {
    if text.contains("query rejected by read-only safety policy") {
        "refused-by-safety"
    } else {
        "error"
    }
}

/// The committed golden's path. `name` is the scenario file stem.
pub(crate) fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/mvp/db_goldens/golden")
        .join(format!("{name}.json"))
}

/// Compares against the committed golden, or rewrites it when
/// `SAYA_UPDATE_GOLDENS=1`. The update is explicit so a behaviour change is
/// reviewed, never silently accepted.
pub(crate) fn assert_golden(name: &str, golden: &Golden) {
    let path = golden_path(name);
    if std::env::var("SAYA_UPDATE_GOLDENS").is_ok_and(|value| value == "1") {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut text = serde_json::to_string_pretty(golden).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing golden {}: {error}. Regenerate with SAYA_UPDATE_GOLDENS=1",
            path.display()
        )
    });
    let committed: Golden = serde_json::from_str(&committed)
        .unwrap_or_else(|error| panic!("golden {} is valid JSON: {error}", path.display()));
    assert_eq!(
        golden,
        &committed,
        "behaviour changed for {}: review the diff, then regenerate with \
         SAYA_UPDATE_GOLDENS=1 only if the change is intended",
        path.display()
    );
}
