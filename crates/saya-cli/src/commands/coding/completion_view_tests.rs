//! The class a coordinator reads next to a failed completion, and its absence everywhere else.

use saya_store::CodingCompletion;
use saya_types::{
    AgentId, CodingFailureClass, CodingResultOutcome, InvocationId, JobId, JoinedResultV1,
};
use serde_json::{Value, json};
use std::num::NonZeroU64;

/// The fuller completion payload, report included.
fn result_view(completion: &CodingCompletion) -> Value {
    with_failure_class(
        json!({
            "sequence": completion.sequence,
            "agent_id": completion.result.agent_id(),
            "outcome": completion.result.outcome(),
            "answer": completion.result.answer(),
            "truncated": completion.result.answer_truncated(),
            "accepted": completion.acknowledged,
            "authority": "none",
        }),
        completion,
    )
}

/// Adds the closed `failure_class` name for a completion that may carry one — `Failed` or
/// `Uncertain` — when it stored one: `Completed` and `Cancelled` never do, and a failure stored
/// before classes existed omits the key. The name is one of a fixed set; no provider or tool text
/// can reach it.
fn with_failure_class(mut view: Value, completion: &CodingCompletion) -> Value {
    if matches!(
        completion.result.outcome(),
        CodingResultOutcome::Failed | CodingResultOutcome::Uncertain
    ) && let Some(class) = completion.result.failure_class()
    {
        view["failure_class"] = json!(class.as_str());
    }
    view
}

fn completion(outcome: CodingResultOutcome, class: Option<CodingFailureClass>) -> CodingCompletion {
    let result = JoinedResultV1::new(
        JobId::parse("job-1").unwrap(),
        AgentId::parse("impl").unwrap(),
        InvocationId::parse("inv-1").unwrap(),
        outcome,
        String::new(),
        false,
    )
    .unwrap();
    CodingCompletion {
        sequence: NonZeroU64::new(7).unwrap(),
        result: match class {
            Some(class) => result.with_failure_class(class),
            None => result,
        },
        acknowledged: false,
    }
}

#[test]
fn a_failed_completion_names_its_closed_failure_class() {
    let item = completion(
        CodingResultOutcome::Failed,
        Some(CodingFailureClass::ProviderStreamEnded),
    );
    let result = result_view(&item);
    assert_eq!(result["failure_class"], "provider_stream_ended");
    assert_eq!(result["outcome"], "failed");
    assert_eq!(result["answer"], "");
}

#[test]
fn an_uncertain_completion_names_its_closed_failure_class() {
    // UCLASS-1: an Uncertain completion may carry a class, and a coordinator
    // reading the completion view must see why it is uncertain.
    let item = completion(
        CodingResultOutcome::Uncertain,
        Some(CodingFailureClass::ExtensionExchange),
    );
    let result = result_view(&item);
    assert_eq!(
        result["failure_class"], "extension_exchange",
        "the view must name the class an Uncertain completion stored: {result}"
    );
    assert_eq!(result["outcome"], "uncertain");
}

#[test]
fn a_completion_that_did_not_fail_carries_no_failure_class_key_even_if_one_was_attached() {
    for outcome in [
        CodingResultOutcome::Completed,
        CodingResultOutcome::Cancelled,
    ] {
        for class in [None, Some(CodingFailureClass::ProviderStreamEnded)] {
            let item = completion(outcome, class);
            let view = result_view(&item);
            assert!(
                view.get("failure_class").is_none(),
                "the view must omit the key for {outcome:?} with {class:?}: {view}"
            );
        }
    }
}

#[test]
fn a_failed_completion_stored_before_classes_existed_omits_the_key_rather_than_null() {
    let item = completion(CodingResultOutcome::Failed, None);
    let view = result_view(&item);
    assert!(view.get("failure_class").is_none(), "{view}");
}
