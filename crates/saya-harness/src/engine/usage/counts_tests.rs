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

    for (events, target) in [
        (
            vec![
                RunEvent::StepStarted { step: 0 },
                counts(Some(0), Some(0)),
                counts(Some(1), None),
                counts(None, Some(2)),
                RunEvent::StepCompleted { step: 0 },
                RunEvent::StepStarted { step: 0 },
                counts(Some(0), Some(0)),
            ],
            0,
        ),
        (
            vec![
                RunEvent::StepStarted { step: 0 },
                counts(Some(0), Some(0)),
                RunEvent::StepCompleted { step: 0 },
                RunEvent::StepStarted { step: 1 },
                counts(Some(0), Some(0)),
                counts(Some(1), None),
                counts(None, Some(1)),
                RunEvent::StepCompleted { step: 1 },
                RunEvent::StepStarted { step: 0 },
                counts(Some(0), Some(0)),
            ],
            0,
        ),
    ] {
        let replayed = StepAnsweringCounts::from_journal(&events, target);
        assert!(
            matches!(replayed, Err(AnsweringCountError::InvalidEvidence)),
            "completed same-index and backward step starts cannot mint a fresh grant: {:?}",
            replayed.map(|counts| counts.remaining(target, Some(3), Some(7)))
        );
    }
}
