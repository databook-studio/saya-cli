//! The goldens' frozen projection: the fields `ref/05` §2 says may never move
//! without the master deciding, compared with a checked-in copy of the goldens
//! as `f7c08884` committed them.
//!
//! A regeneration moves the *unfrozen* fields only — the advertised tool names
//! and order, the normalised body sizes. Everything here is the behaviour the
//! release must keep: what the run called, the state every turn settled to, the
//! message count of every request, the recall each request carried, and what the
//! run learned or replayed. The copy in `frozen/f7c08884/` is what those fields
//! were before the refactor, so a regeneration that moved one of them fails here
//! and stays un-regenerated until the master accepts the behaviour change.
//!
//! One row is re-specified rather than compared: `question_budget_stop`, whose
//! scenario `ref/05` §3 re-states (a database-armed stop, then the run's next
//! line). Its frozen projection is the one that was *meant* to move, and
//! [`the_re_specified_budget_row_is_what_ref05_section_3_states`] is where it is
//! held to that shape.

use std::path::PathBuf;

use super::record::{Golden, golden_path};

/// Where the `f7c08884` copies live, beside this file.
fn frozen_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mvp/db_goldens/frozen/f7c08884")
}

fn frozen_path(name: &str) -> PathBuf {
    frozen_dir().join(format!("{name}.json"))
}

/// The rows compared field for field against their `f7c08884` copy.
const COMPARED: &[&str] = &[
    "plain_question",
    "clarification",
    "write_refused",
    "recall_injected",
    "learning_written",
    "contract_search",
    "investigation",
];

/// The one row `ref/05` §3 re-specifies: its projection may move, and only as
/// that section states.
const RE_SPECIFIED: &str = "question_budget_stop";

/// The projection that may not move: the calls the run made, the state each turn
/// settled to, and what each request carried of the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frozen {
    tool_calls: Vec<super::record::ToolCallRecord>,
    states: Vec<super::record::StateRecord>,
    messages: Vec<usize>,
    recall: Vec<Option<super::record::RecallRecord>>,
    learning: Option<super::record::LearningRecord>,
    investigation: Option<super::record::InvestigationRecord>,
}

impl Frozen {
    fn of(golden: &Golden) -> Self {
        Self {
            tool_calls: golden.tool_calls.clone(),
            states: golden.states.clone(),
            messages: golden
                .requests
                .iter()
                .map(|request| request.messages)
                .collect(),
            recall: golden
                .requests
                .iter()
                .map(|request| request.recall.clone())
                .collect(),
            learning: golden.learning.clone(),
            investigation: golden.investigation.clone(),
        }
    }
}

fn read(path: &PathBuf) -> Golden {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("golden {} reads: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("golden {} is valid JSON: {error}", path.display()))
}

#[test]
fn every_goldens_frozen_projection_is_the_one_f7c08884_committed() {
    for name in COMPARED {
        let committed = read(&golden_path(name));
        let before = read(&frozen_path(name));
        assert_eq!(
            Frozen::of(&committed),
            Frozen::of(&before),
            "the frozen projection of `{name}` moved since f7c08884: calls, settled \
             states, message counts, recall, learning and investigation may not change \
             without the master deciding it"
        );
    }
}

/// `ref/05` §3: the stop is on `[limits.database]`, armed by the turn's own
/// database call, and the run's next line is a turn of the same run. So the row's
/// projection is the re-specified shape: one database call, then a stop and an
/// answer, over three requests.
#[test]
fn the_re_specified_budget_row_is_what_ref05_section_3_states() {
    let golden = read(&golden_path(RE_SPECIFIED));
    let before = read(&frozen_path(RE_SPECIFIED));
    assert_ne!(
        Frozen::of(&golden),
        Frozen::of(&before),
        "`{RE_SPECIFIED}` is the one row §3 re-specifies, so its projection is \
         expected to differ from f7c08884's"
    );

    let calls: Vec<(&str, &str)> = golden
        .tool_calls
        .iter()
        .map(|call| (call.name.as_str(), call.outcome.as_str()))
        .collect();
    assert_eq!(
        calls,
        vec![("schema_discovery", "ok")],
        "the database call that armed the budget, and nothing else"
    );
    let states: Vec<(&str, &str)> = golden
        .states
        .iter()
        .map(|state| (state.kind.as_str(), state.text.as_str()))
        .collect();
    assert_eq!(
        states,
        vec![
            (
                "budget-stop",
                "the run stopped on its elapsed-time budget; any answer it produced is \
                 partial or absent"
            ),
            ("answer", "the-final-answer"),
        ],
        "turn 1 stopped on the budget, turn 2 answered"
    );
    assert_eq!(
        golden.requests.len(),
        3,
        "turn 1's two requests and turn 2's: {:?}",
        golden
            .requests
            .iter()
            .map(|request| request.messages)
            .collect::<Vec<_>>()
    );
    assert_eq!(golden.learning, None, "no extraction on this row");
    assert_eq!(golden.investigation, None, "no replay on this row");
}
