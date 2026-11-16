//! Native context delivery, distinct from initial child input and storage retention.
#![cfg(unix)]
#[path = "coding_context_efficiency/compaction.rs"]
mod compaction;
#[path = "coding_context_efficiency/evidence.rs"]
mod evidence;
#[path = "coding_context_efficiency/fixture.rs"]
mod fixture;
#[path = "coding_context_efficiency/provider.rs"]
mod provider;
#[path = "coding_context_efficiency/script.rs"]
mod script;
#[path = "coding_context_efficiency/windows.rs"]
mod windows;

use fixture::Fixture;
use provider::Provider;
use saya_agent::NoopEventSink;
use script::Scenario;
use serde_json::Value;
use std::time::Duration;

/// Load ceiling, not a product bound: how long a test waits for a scripted job
/// that finishes on its own. The scripted runs here execute dozens of turns, so
/// the same run takes several times longer on a machine that is also running
/// the rest of the suite; a wait under this ceiling still ends the moment the
/// job returns.
pub(crate) const RUN_CEILING: Duration = Duration::from_secs(900);

/// Load ceiling, not a product bound: how long the scripted provider keeps
/// serving. It must outlast `RUN_CEILING`, or a slow run would fail on a
/// shut-down provider instead of on the run ceiling.
pub(crate) const PROVIDER_LIFETIME: Duration = Duration::from_secs(1_200);

#[tokio::test]
async fn native_coding_context_respects_resolved_budget() {
    for (budget, batches, width) in [
        (256 * 1024, 1, 4),
        (4 * 1024 * 1024, 4, 16),
        (64 * 1024, 1, 4),
    ] {
        let provider = Provider::new(Scenario::Feedback { batches, width });
        let fixture =
            Fixture::new(&provider.endpoint, budget, Some(8_000_000), "child-context").await;
        let job = fixture.create().await;
        let output = tokio::time::timeout(RUN_CEILING, job.run(&NoopEventSink))
            .await
            .unwrap()
            .unwrap();
        assert!(!output.truncated);
        let bodies = provider.bodies();
        let child = evidence::child_requests(&bodies);
        assert!(child.iter().all(|body| body["model"] == "child-context"));
        assert!(
            bodies
                .iter()
                .all(|body| body["max_completion_tokens"] == 4096)
        );
        assert!(
            child
                .iter()
                .all(|body| evidence::content_bytes(body) <= budget)
        );
        let results = evidence::results(&child);
        // Every configured budget, the smallest included, delivers the actual
        // file: the per-call result cap is `min(budget, MAX_TOOL_MESSAGE_BYTES)`
        // and the fixture files sit under it, so a smaller budget trims history,
        // never the read the model asked for.
        for batch in 0..batches {
            for index in 0..width {
                let id = format!("read-{batch}-{index}");
                let result = results
                    .get(&id)
                    .unwrap_or_else(|| panic!("missing actual read {id}"));
                let value: Value = serde_json::from_str(result).unwrap_or_else(|_| {
                    panic!("actual file feedback truncated at configured {budget}: {id}")
                });
                assert_eq!(
                    evidence::unnumbered(value["content"].as_str().unwrap()),
                    fixture.file_body(index)
                );
                // A false result flag is left out of the shape, so an absent
                // `truncated` reads as that default: the read was not clipped.
                assert!(
                    !value["truncated"].as_bool().unwrap_or(false),
                    "a read that fits its budget is not truncated: {value}"
                );
                assert!(
                    value["digest"]
                        .as_str()
                        .is_some_and(|digest| digest.len() == 64)
                );
            }
        }
        if budget == 4 * 1024 * 1024 {
            assert!(
                child
                    .iter()
                    .any(|body| evidence::content_bytes(body) > 256 * 1024),
                "4MiB must retain actual multi-turn history beyond 256KiB"
            );
        }
        evidence::healthy(&fixture, &job, batches * width + 1).await;
        evidence::reviewed(&fixture, &job).await;
    }
}
