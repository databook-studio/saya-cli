//! Owner-state controls; public-loop tests separately prove scheduler wiring.
use super::{AuthorityRefusalContext, CodingCheckpoint, Observation, observe};
use saya_types::InvocationId;
use serde_json::{Value, json};

fn file(path: &str, digest: char, name: &str) -> Observation {
    observe(
        name,
        &json!({"path":path}),
        Ok(&json!({"path":path,"size":8,"digest":digest.to_string().repeat(64)})),
    )
}

#[test]
fn authority_refusals_are_scoped_to_one_invocation_and_hide_private_paths() {
    let context = AuthorityRefusalContext::new(InvocationId::parse("invocation-a").unwrap(), 7);
    let mut first = CodingCheckpoint::with_authority(Some(context));
    first.observe_authority_denial("write", "write");
    let first = projection(&first).to_string();
    assert!(!first.contains("invocation-a"));
    assert!(first.contains("report_to_coordinator"));
    assert!(!first.contains("path_identity"));

    let next = CodingCheckpoint::with_authority(Some(AuthorityRefusalContext::new(
        InvocationId::parse("invocation-b").unwrap(),
        8,
    )));
    assert!(
        next.render(16 * 1024).is_empty(),
        "new authority starts clean"
    );
}
fn projection(checkpoint: &CodingCheckpoint) -> Value {
    serde_json::from_str(&checkpoint.render(16 * 1024)).unwrap()
}

#[test]
fn follow_up_keeps_bounded_owner_receipts_without_raw_child_data() {
    let mut checkpoint = CodingCheckpoint::default();
    let follow_up = json!({
        "agent_id":"agent-a",
        "previous_invocation_id":"invocation-before",
        "invocation_id":"invocation-after",
        "previous_completion_sequence":41,
        "checkout_id":"checkout-a",
        "diff":"RAW_CHILD_DIFF_SENTINEL"
    });
    checkpoint.observe_serial(&observe("follow_up_child", &json!({}), Ok(&follow_up)));

    let projected = projection(&checkpoint);
    let facts = projected["coding_facts"].as_array().unwrap();
    assert!(facts.iter().any(|fact| {
        fact["kind"] == "follow_up"
            && fact["previous_invocation_id"] == "invocation-before"
            && fact["invocation_id"] == "invocation-after"
            && fact["previous_completion_sequence"] == 41
            && fact["checkout_id"] == "checkout-a"
    }));
    assert!(!projected.to_string().contains("RAW_CHILD_DIFF_SENTINEL"));
}

#[test]
fn conflicting_overlapping_file_results_do_not_choose_a_latest_identity() {
    let mut checkpoint = CodingCheckpoint::default();
    checkpoint.observe_serial(&file("a.rs", 'a', "read"));
    let before = file("a.rs", 'a', "read");
    let after = file("a.rs", 'b', "read");
    checkpoint.observe_batch(&[&before, &after]);
    let projected = projection(&checkpoint);
    assert!(
        projected["coding_facts"].as_array().unwrap().is_empty(),
        "conflicting observations choose no winner"
    );
    assert!(projected["omitted_facts"].as_u64().unwrap() > 0);
    let reversed = {
        let mut checkpoint = CodingCheckpoint::default();
        checkpoint.observe_serial(&file("a.rs", 'a', "read"));
        checkpoint.observe_batch(&[&after, &before]);
        projection(&checkpoint)
    };
    assert_eq!(projected["coding_facts"], reversed["coding_facts"]);
}

#[test]
fn redaction_omits_identity_but_the_real_mutation_still_lands() {
    let mut checkpoint = CodingCheckpoint::default();
    checkpoint.observe_serial(&file("a.rs", 'a', "read"));
    let mutation = file("api_key=sk-live-PATH_SENTINEL.rs", 'a', "write");
    checkpoint.observe_serial(&mutation);
    let projected = projection(&checkpoint);
    assert_eq!(projected["coding_facts"].as_array().unwrap().len(), 1);
    assert_eq!(projected["coding_facts"][0]["path"], "a.rs");
    assert_eq!(projected["omitted_facts"], 1);
    assert!(!projected.to_string().contains("PATH_SENTINEL"));
    assert!(!projected.to_string().contains("[redacted]"));
    let mut batch = CodingCheckpoint::default();
    batch.observe_serial(&file("a.rs", 'a', "read"));
    batch.observe_batch(&[&mutation]);
    assert_eq!(projection(&batch)["coding_facts"][0]["path"], "a.rs");
}

#[test]
fn cumulative_history_evicts_whole_old_facts_at_count_and_byte_bounds() {
    for wide in [false, true] {
        let mut checkpoint = CodingCheckpoint::default();
        let prefix = if wide {
            "nested/".repeat(60)
        } else {
            String::new()
        };
        for index in 0..100 {
            checkpoint.observe_serial(&file(&format!("{prefix}file-{index}.rs"), 'a', "read"));
        }
        let rendered = checkpoint.render(16 * 1024);
        assert!(rendered.len() <= 16 * 1024);
        let projected: Value = serde_json::from_str(&rendered).unwrap();
        let facts = projected["coding_facts"].as_array().unwrap();
        assert!(facts.len() <= 64);
        assert!(projected["omitted_facts"].as_u64().unwrap() > 0);
        assert!(
            facts
                .iter()
                .any(|fact| fact["path"] == format!("{prefix}file-99.rs"))
        );
        assert!(
            !facts
                .iter()
                .any(|fact| fact["path"] == format!("{prefix}file-0.rs"))
        );
        for fact in facts {
            assert!(
                saya_types::WorkspacePath::try_from(fact["path"].as_str().unwrap().to_owned())
                    .is_ok()
            );
            assert_eq!(fact["digest"], "a".repeat(64));
        }
        let tiny = checkpoint.render(180);
        assert!(tiny.len() <= 180);
        assert!(
            tiny.is_empty()
                || serde_json::from_str::<Value>(&tiny).unwrap()["omitted_facts"]
                    .as_u64()
                    .unwrap()
                    > 0
        );
    }
}

// A successful ToolExecutor mutation need not publish optional identity fields.
// Existing controls cover complete identities and Err, not safe omission on Ok.
// Exercise extraction -> state update -> projection without a production seam.
#[test]
fn successful_mutations_with_omitted_identity_still_land_without_fabricating_identity() {
    for name in ["read", "write", "edit"] {
        for value in [
            json!({}),
            json!({"path":"a.rs","size":8}),
            json!({"path":"other.rs","size":8,"digest":"a".repeat(64)}),
        ] {
            let observation = observe(name, &json!({"path":"a.rs"}), Ok(&value));
            assert!(
                observation.mutation_path.is_none(),
                "no confirmed retry witness"
            );
            let mut serial = CodingCheckpoint::default();
            serial.observe_serial(&observation);
            let mut batch = CodingCheckpoint::default();
            batch.observe_batch(&[&observation]);
            for checkpoint in [serial, batch] {
                let projected = projection(&checkpoint);
                assert!(
                    projected["coding_facts"].as_array().unwrap().is_empty(),
                    "omitted metadata never invents file identity"
                );
                assert_eq!(projected["omitted_facts"], 1);
            }
        }
    }
}
