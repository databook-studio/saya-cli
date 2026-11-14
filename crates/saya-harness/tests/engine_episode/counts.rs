use super::*;

#[path = "counts/append_failure.rs"]
mod append_failure;
#[path = "counts/fresh_grant.rs"]
mod fresh_grant;
#[path = "counts/provider_failures.rs"]
mod provider_failures;
#[path = "counts/replay.rs"]
mod replay;
#[path = "counts/retry.rs"]
mod retry;

use saya_types::RunEvent;

fn count_pairs_for_step(events: &[RunEvent], step: usize) -> Vec<(Option<u64>, Option<u64>)> {
    let Some(start) = events
        .iter()
        .position(|event| matches!(event, RunEvent::StepStarted { step: found } if *found == step))
    else {
        return Vec::new();
    };
    events[start + 1..]
        .iter()
        .take_while(
            |event| !matches!(event, RunEvent::StepStarted { step: found } if *found != step),
        )
        .filter_map(|event| match event {
            RunEvent::Usage {
                tokens: None,
                turns,
                tool_calls,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
                ..
            } if turns.is_some() || tool_calls.is_some() => Some((*turns, *tool_calls)),
            _ => None,
        })
        .collect()
}
