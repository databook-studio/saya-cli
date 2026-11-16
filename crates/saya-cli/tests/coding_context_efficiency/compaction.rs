use super::{
    RUN_CEILING, evidence,
    fixture::Fixture,
    provider::Provider,
    script::{COMPACTION_READS_PER_BATCH as READS_PER_BATCH, Scenario},
};
use saya_agent::NoopEventSink;
use serde_json::Value;
use std::fs;

/// Read batches the child issues. Long enough that the 256 KiB history below
/// compacts at least twice, so the facts are shown to survive successive
/// compactions; the compaction-count assertion fails if it ever is not.
const BATCHES: usize = 48;

#[tokio::test]
async fn native_larger_context_preserves_cumulative_coding_facts() {
    let provider = Provider::new(Scenario::Compaction { batches: BATCHES });
    let fixture = Fixture::new(
        &provider.endpoint,
        256 * 1024,
        Some(8_000_000),
        "child-context",
    )
    .await;
    let job = fixture.create().await;
    let output = tokio::time::timeout(RUN_CEILING, job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(!output.truncated);
    let journal =
        fs::read_to_string(fixture.root.join("coding/context-job/compactions.jsonl")).unwrap();
    let compactions = journal
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["agent_id"] == "child-context")
        .collect::<Vec<_>>();
    assert!(
        compactions.len() >= 2,
        "actual native child compactions: {}",
        compactions.len()
    );
    assert!(
        compactions
            .iter()
            .all(|record| record["observation"]["protected_prefix_unchanged"] == true)
    );
    let bodies = provider.bodies();
    let child = evidence::child_requests(&bodies);
    let results = evidence::results(&child);
    let observed_reads = results.keys().filter(|id| id.starts_with("read-")).count();
    assert_eq!(
        observed_reads,
        BATCHES * READS_PER_BATCH,
        "all read call IDs were returned"
    );
    // Every read is executed and returns the actual file, the identical
    // repeats included: the per-call result cap is `min(budget, 64 KiB)` and
    // the fixture files sit under it, so compaction trims history without ever
    // refusing or mangling the read the child asked for.
    for batch in 0..BATCHES {
        for index in 0..READS_PER_BATCH {
            let id = format!("read-{batch}-{index}");
            let text = results
                .get(&id)
                .unwrap_or_else(|| panic!("missing actual read result {id}"));
            let value: Value = serde_json::from_str(text)
                .unwrap_or_else(|_| panic!("read result is not valid JSON: {id}: {text}"));
            evidence::assert_valid_read_value(&value, &fixture, index, &id);
        }
    }
    let write: Value = serde_json::from_str(&results["write"]).unwrap();
    let mut final_facts = None;
    for body in &child {
        for message in body["messages"].as_array().unwrap() {
            let Some(text) = message["content"].as_str() else {
                continue;
            };
            if message["role"] != "assistant"
                || message.get("tool_calls").is_some()
                || !text.starts_with("…[compacted:")
            {
                continue;
            }
            assert!(text.len() <= 16 * 1024);
            for forbidden in [
                "PRIVATE_FILE_BODY",
                "owned context change",
                "environment_profile",
                "toolchain",
                "stdout",
                "stderr",
            ] {
                assert!(
                    !text.contains(forbidden),
                    "private body/runtime entered checkpoint"
                );
            }
            let facts: Value = serde_json::from_str(
                text.lines()
                    .find(|line| line.starts_with("{\"authority\""))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(facts["authority"], "none");
            assert!(facts["coding_facts"].as_array().unwrap().len() <= 64);
            final_facts = Some(facts);
        }
    }
    let facts = final_facts.expect("generated cumulative checkpoint after actual compactions");
    let retained = facts["coding_facts"].as_array().unwrap();
    assert!(
        retained.iter().any(|fact| fact["kind"] == "file"
            && fact["path"] == "new.txt"
            && fact["digest"] == write["digest"]),
        "early actual write identity retained"
    );
    // Every read is executed and charged, the repeats included: only the
    // child's own write sits beside them.
    evidence::healthy(&fixture, &job, 1 + observed_reads).await;
    evidence::reviewed(&fixture, &job).await;
}
