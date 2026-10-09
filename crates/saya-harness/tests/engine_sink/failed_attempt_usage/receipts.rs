//! The durable record's assertions: exactly one `Usage` event per actual
//! settled receipt carrying the figures that receipt reported, the carried
//! total counting both once, and neither the executor nor the provider
//! having done anything beyond the scripted calls.

use saya_harness::engine::UsageTotals;
use saya_harness::journal::Journal;
use saya_types::RunEvent;

use super::fixtures::{NoTools, ScriptedProvider};

pub(super) fn assert_actual_receipts(
    journal: &Journal,
    tools: &NoTools,
    provider: &ScriptedProvider,
) {
    let events = journal.read().unwrap();
    let usage = events
        .iter()
        .filter(|event| matches!(event, RunEvent::Usage { .. }))
        .collect::<Vec<_>>();
    assert_eq!(usage.len(), 2, "one per actual settled receipt: {events:?}");
    assert!(matches!(
        usage[0],
        RunEvent::Usage {
            tokens: Some(30),
            cached_input_tokens: Some(5),
            cache_creation_input_tokens: None,
            ..
        }
    ));
    assert!(matches!(
        usage[1],
        RunEvent::Usage {
            tokens: Some(14),
            cached_input_tokens: None,
            cache_creation_input_tokens: Some(0),
            ..
        }
    ));
    assert_eq!(
        UsageTotals::from_journal(&events).carried_tokens,
        44,
        "the successful usage and failed attempt each count once"
    );
    assert_eq!(tools.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(
        provider.requests.load(std::sync::atomic::Ordering::Relaxed),
        3
    );
}
