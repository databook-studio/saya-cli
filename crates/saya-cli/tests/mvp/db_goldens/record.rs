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

use super::classify::{classify_result, record_tool_result};
use super::harness::ServedCall;
use super::launch_presence;
use super::launch_summary;
use super::normalise::Normaliser;
use super::record_b::{is_extraction_request, recall_block};

/// One scenario's committed golden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Golden {
    pub(crate) scenario: String,
    pub(crate) requests: Vec<RequestRecord>,
    pub(crate) tool_calls: Vec<ToolCallRecord>,
    /// One entry per settled turn, in the order the user saw them.
    pub(crate) states: Vec<StateRecord>,
    /// Post-turn extraction, present only when the run made one. Absent from
    /// the part-A goldens (memory is off there), so their bytes do not move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) learning: Option<LearningRecord>,
    /// A saved-investigation replay, present only for the no-model-loop row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) investigation: Option<InvestigationRecord>,
}

/// What one provider request advertised and carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RequestRecord {
    /// The advertised tool names, in wire order.
    pub(crate) tools: Vec<String>,
    /// The body's byte length after normalisation, with the launch summary's
    /// block taken out: whether the background scan landed before the turn's
    /// first request is a race (`launch_presence`), so the bytes that race
    /// decides must not be in the number.
    pub(crate) body_bytes: usize,
    pub(crate) messages: usize,
    /// The recall block this request carried, if any. Absent from the part-A
    /// goldens (memory is off there), so their bytes do not move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recall: Option<RecallRecord>,
    /// How many times this request carries the launch summary's block: `0` or
    /// `1`, the run's one launch fact. Recorded, and set aside before two
    /// records are compared, because whether it is there at all is the race.
    #[serde(default)]
    pub(crate) launch_summary_count: usize,
}

/// The prompt-recall context block a request carried, measured after
/// normalisation. The block's text is never recorded — only that it is there
/// and how many normalised bytes it occupies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecallRecord {
    /// True when the request carried a `database-contracts` block.
    pub(crate) present: bool,
    /// Bytes between the block's `source:` line and its closing sentinel.
    pub(crate) body_bytes: usize,
}

/// Post-turn learning as the run performed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LearningRecord {
    /// Provider requests carrying the extractor's JSON-mode shape.
    pub(crate) extraction_requests: usize,
    /// Claims the scripted extractor reply offered.
    pub(crate) proposed: usize,
    /// `memory learned` / `memory noted` lines the user saw.
    pub(crate) stored: usize,
}

/// A saved-investigation replay: no model loop, so `requests` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InvestigationRecord {
    /// The statements the replay attempted, in order.
    pub(crate) statements: Vec<StatementRecord>,
    /// The user-visible result event of each replay (`query_result`, `error`).
    pub(crate) results: Vec<String>,
}

/// One statement a replay attempted and its safety outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StatementRecord {
    pub(crate) sql: String,
    /// `ok`, `refused-by-safety`, or `error`.
    pub(crate) outcome: String,
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
        let mut extraction_requests = 0;

        for body in bodies {
            let normalised = normaliser.normalise(body);
            let value: serde_json::Value = serde_json::from_str(&normalised)
                .unwrap_or_else(|error| panic!("request body is JSON: {error}; {normalised}"));
            let messages = value["messages"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            if is_extraction_request(&value) {
                extraction_requests += 1;
            }
            requests.push(RequestRecord {
                tools: advertised_tools(&value),
                body_bytes: launch_summary::bytes_without_launch_summary(&normalised, &value),
                messages: messages.len(),
                recall: recall_block(&value),
                launch_summary_count: launch_summary::recorded_count(&normalised, &value, scenario),
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
            learning: (extraction_requests > 0).then_some(LearningRecord {
                extraction_requests,
                proposed: 0,
                stored: 0,
            }),
            investigation: None,
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
    launch_presence::assert_rule(name, &golden.requests);
    launch_presence::assert_rule(name, &committed.requests);
    assert_eq!(
        golden.without_launch_presence(),
        committed.without_launch_presence(),
        "behaviour changed for {}: review the diff, then regenerate with \
         SAYA_UPDATE_GOLDENS=1 only if the change is intended",
        path.display()
    );
}
