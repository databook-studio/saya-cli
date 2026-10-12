//! JSON summary rendering for diagnostic delta states.

use super::DiagnosticDeltaState;
use serde_json::{Value, json};

pub(in crate::commands::coding) fn summary(states: &[Option<DiagnosticDeltaState>]) -> Value {
    let states = states.iter().filter_map(Option::as_ref).collect::<Vec<_>>();
    if states.is_empty() {
        return json!({"available":false,"reason":"comparable_baseline_not_minted"});
    }
    if states.iter().all(|state| state.available().is_some()) {
        let deltas = states
            .iter()
            .filter_map(|state| state.available())
            .collect::<Vec<_>>();
        return json!({
            "available":true,
            "introduced_count":deltas.iter().map(|delta|delta.introduced().len()).sum::<usize>(),
            "unchanged_count":deltas.iter().map(|delta|delta.unchanged().len()).sum::<usize>(),
            "resolved_count":deltas.iter().map(|delta|delta.resolved().len()).sum::<usize>()
        });
    }
    let reason = states
        .iter()
        .find_map(|state| match state {
            DiagnosticDeltaState::BaselinePending { committed, .. } => Some(if *committed {
                "baseline_minted_for_future_review"
            } else {
                "baseline_pending_publication"
            }),
            DiagnosticDeltaState::Unavailable(reason) => Some(*reason),
            _ => None,
        })
        .unwrap_or("baseline_unavailable");
    json!({"available":false,"reason":reason})
}

pub(super) fn entries(entries: &[saya_harness::coding::checks::Diagnostic]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| {
            json!({
                "severity":format!("{:?}",entry.severity()),
                "code":entry.code(),
                "message":saya_types::redact(entry.message()),
                "locations":entry.locations().iter().map(|location|json!({
                    "path":location.path(),
                    "line":location.line_start(),
                    "column":location.column_start()
                })).collect::<Vec<_>>()
            })
        })
        .collect()
}
