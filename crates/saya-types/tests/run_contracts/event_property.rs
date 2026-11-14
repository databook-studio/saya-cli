use super::*;
use proptest::prelude::*;

proptest! {
    /// Every `RunEvent` serializes to exactly one NDJSON line — the string
    /// `serde_json` produces never contains a raw newline — and every line
    /// parses back to the event it came from. The run journal and the
    /// `--format ndjson` wire both depend on this.
    #[test]
    fn event_is_one_line_and_round_trips(
        endpoint in "[a-z0-9_-]{1,32}",
        tokens in prop::option::of(0u64..1_000_000),
        turns in prop::option::of(0u64..1_000),
        tool_calls in prop::option::of(0u64..1_000),
        cached in prop::option::of(0u64..1_000_000),
        cache_creation in prop::option::of(0u64..1_000_000),
        step_index in 0usize..64,
        reason in prop::sample::select(vec![
            PauseReason::BudgetExhausted,
            PauseReason::WallClockExceeded,
            PauseReason::StepFailedAfterRetry,
            PauseReason::StoreUnavailable,
            PauseReason::ProcessDeath,
            PauseReason::UserPaused,
        ]),
        code in prop::sample::select(vec![
            RunFailureCode::SafetyQuery,
            RunFailureCode::Provider,
            RunFailureCode::ConnectionConfig,
        ]),
        bytes in 0u64..1_000_000_000,
        kind in prop::sample::select(vec![0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ) {
        let event = match kind {
            0 => RunEvent::RunStarted,
            1 => RunEvent::PlanApproved {
                scopes: Some(vec![format!("fetch:https+{endpoint}")]),
            },
            2 => RunEvent::StepStarted { step: step_index },
            3 => RunEvent::StepCompleted { step: step_index },
            4 => RunEvent::StepFailed { step: step_index },
            5 => RunEvent::Deliverables {
                step: step_index,
                entries: vec![
                    Deliverable::present("report.md", 29, "4634"),
                    Deliverable::missing("draft.md"),
                ],
            },
            6 => RunEvent::Paused { reason },
            7 => RunEvent::Completed,
            8 => RunEvent::Failed { code },
            9 => RunEvent::Cancelled,
            11 => RunEvent::DownloadedBytes { bytes },
            _ => RunEvent::Usage {
                endpoint: endpoint.clone(),
                tokens,
                turns,
                tool_calls,
                cached_input_tokens: cached,
                cache_creation_input_tokens: cache_creation,
            },
        };
        let line = serde_json::to_string(&event).unwrap();
        prop_assert!(!line.contains('\n'), "event spanned multiple lines: {line}");
        let back: RunEvent = serde_json::from_str(&line).unwrap();
        prop_assert_eq!(back, event);
    }
}
