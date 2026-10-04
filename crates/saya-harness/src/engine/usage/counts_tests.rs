use super::{AnsweringCountError, StepAnsweringCounts};
use saya_types::RunEvent;

fn counts(turns: Option<u64>, tool_calls: Option<u64>) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: None,
        turns,
        tool_calls,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

#[test]
fn replay_keeps_pending_and_malformed_count_evidence_unknown() {
    let pending = StepAnsweringCounts::from_journal(
        &[
            RunEvent::StepStarted { step: 4 },
            counts(Some(0), Some(0)),
            counts(Some(1), None),
        ],
        4,
    )
    .unwrap();
    assert_eq!(
        pending.remaining(4, Some(3), Some(7)),
        Err(AnsweringCountError::PendingBatch),
        "an admitted response without a completed batch cannot be treated as zero calls"
    );

    let malformed = StepAnsweringCounts::from_journal(
        &[
            RunEvent::StepStarted { step: 4 },
            counts(Some(0), Some(0)),
            counts(Some(2), None),
        ],
        4,
    );
    assert!(
        matches!(malformed, Err(AnsweringCountError::InvalidEvidence)),
        "unsupported count pairs fail closed instead of reseeding the grant"
    );
}
