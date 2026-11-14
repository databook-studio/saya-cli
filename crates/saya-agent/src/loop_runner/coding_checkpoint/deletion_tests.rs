//! Actual observation -> bounded context contracts for deletion; no filesystem seam.
use super::{CodingCheckpoint, observe};
use crate::ToolError;
use serde_json::{Value, json};

fn seeded() -> CodingCheckpoint {
    let mut checkpoint = CodingCheckpoint::default();
    checkpoint.observe_serial(&observe(
        "read",
        &json!({"path":"a.rs"}),
        Ok(&json!({"path":"a.rs","size":8,"digest":"a".repeat(64)})),
    ));
    checkpoint
}
/// An empty checkpoint renders no digest at all, so the canonical empty
/// projection stands in for it.
fn projection(checkpoint: &CodingCheckpoint) -> Value {
    match checkpoint.render(16 * 1024) {
        rendered if rendered.is_empty() => {
            json!({"authority":"none","coding_facts":[],"omitted_facts":0})
        }
        rendered => serde_json::from_str(&rendered).unwrap(),
    }
}
fn arguments(path: &str) -> Value {
    json!({"path":path,"expected_digest":"a".repeat(64)})
}
fn removed(path: &str) -> Value {
    json!({"path":path,"removed":true,"removed_digest":"a".repeat(64)})
}

#[test]
fn confirmed_deletion_replaces_existing_identity_and_never_grants_creation_retry() {
    let observation = observe("delete", &arguments("a.rs"), Ok(&removed("a.rs")));
    assert!(
        observation.mutation_path.is_none(),
        "deletion cannot witness file creation"
    );
    let mut checkpoint = seeded();
    checkpoint.observe_serial(&observation);
    let view = projection(&checkpoint);
    let facts = view["coding_facts"].as_array().unwrap();
    assert!(!facts.iter().any(|fact| fact["kind"] == "file"));
    let deletion = facts
        .iter()
        .find(|fact| fact["kind"] == "deleted")
        .expect("confirmed tombstone");
    assert_eq!(deletion["path"], "a.rs");
    assert_eq!(deletion["removed_digest"], "a".repeat(64));
    assert!(deletion.get("digest").is_none() && deletion.get("size").is_none());
}

#[test]
fn uncertain_deletion_discards_old_identity_without_claiming_absence_or_retry() {
    for no_effect in [false, true] {
        let inner = ToolError::WorkspaceWrite("owner failure".into());
        let error = if no_effect {
            ToolError::NoEffect(Box::new(inner))
        } else {
            inner
        };
        let observation = observe("delete", &arguments("a.rs"), Err(&error));
        assert!(observation.mutation_path.is_none());
        for batch in [false, true] {
            let mut checkpoint = seeded();
            if batch {
                checkpoint.observe_batch(&[&observation]);
            } else {
                checkpoint.observe_serial(&observation);
            }
            let view = projection(&checkpoint);
            let facts = view["coding_facts"].as_array().unwrap();
            assert!(!facts.iter().any(|fact| fact["kind"] == "deleted"));
            assert_eq!(facts.iter().any(|fact| fact["kind"] == "file"), no_effect);
        }
    }
}

#[test]
fn malformed_or_private_deletion_results_never_fabricate_tombstones() {
    let private = "api_key=sk-live-PATH_SENTINEL.rs";
    for (path, value) in [
        ("a.rs", json!({})),
        ("a.rs", removed("other.rs")),
        (
            "a.rs",
            json!({"path":"a.rs","removed":false,"removed_digest":"a".repeat(64)}),
        ),
        (
            "a.rs",
            json!({"path":"a.rs","removed":true,"removed_digest":"b".repeat(64)}),
        ),
        (private, removed(private)),
    ] {
        let observation = observe("delete", &arguments(path), Ok(&value));
        assert!(observation.mutation_path.is_none());
        let mut checkpoint = seeded();
        checkpoint.observe_serial(&observation);
        let view = projection(&checkpoint);
        let facts = view["coding_facts"].as_array().unwrap();
        assert!(!facts.iter().any(|fact| fact["kind"] == "deleted"));
        assert!(!view.to_string().contains("PATH_SENTINEL"));
        assert!(view["omitted_facts"].as_u64().unwrap() > 0);
    }
}

#[test]
fn overlapping_read_and_delete_conflict_in_both_collection_orders() {
    let read = observe(
        "read",
        &json!({"path":"a.rs"}),
        Ok(&json!({"path":"a.rs","size":8,"digest":"a".repeat(64)})),
    );
    let delete = observe("delete", &arguments("a.rs"), Ok(&removed("a.rs")));
    for observations in [[&read, &delete], [&delete, &read]] {
        let mut checkpoint = seeded();
        checkpoint.observe_batch(&observations);
        let view = projection(&checkpoint);
        let facts = view["coding_facts"].as_array().unwrap();
        assert!(!facts.iter().any(|fact| fact["path"] == "a.rs"));
        assert!(view["omitted_facts"].as_u64().unwrap() > 0);
    }
}

#[test]
fn overlapping_read_and_uncertain_deletion_never_restore_a_current_file_identity() {
    let read = observe(
        "read",
        &json!({"path":"a.rs"}),
        Ok(&json!({"path":"a.rs","size":8,"digest":"a".repeat(64)})),
    );
    let error = ToolError::WorkspaceWrite("owner outcome unknown".into());
    let delete = observe("delete", &arguments("a.rs"), Err(&error));
    for observations in [[&read, &delete], [&delete, &read]] {
        let mut checkpoint = seeded();
        checkpoint.observe_batch(&observations);
        let view = projection(&checkpoint);
        let facts = view["coding_facts"].as_array().unwrap();
        assert!(!facts.iter().any(|fact| fact["path"] == "a.rs"));
        assert!(view["omitted_facts"].as_u64().unwrap() > 0);
    }
}

#[test]
fn possible_recreation_invalidates_absence_serially_and_in_overlapping_batches() {
    let delete = observe("delete", &arguments("a.rs"), Ok(&removed("a.rs")));
    for name in ["write", "edit"] {
        for known_no_effect in [false, true] {
            let inner = ToolError::WorkspaceWrite("owner outcome unknown".into());
            let error = if known_no_effect {
                ToolError::NoEffect(Box::new(inner))
            } else {
                inner
            };
            let mutation = observe(name, &json!({"path":"a.rs"}), Err(&error));
            for batch in [false, true] {
                for reversed in [false, true] {
                    let mut checkpoint = seeded();
                    if batch {
                        let observations = if reversed {
                            [&mutation, &delete]
                        } else {
                            [&delete, &mutation]
                        };
                        checkpoint.observe_batch(&observations);
                    } else {
                        checkpoint.observe_serial(&delete);
                        checkpoint.observe_serial(&mutation);
                    }
                    let view = projection(&checkpoint);
                    let facts = view["coding_facts"].as_array().unwrap();
                    assert_eq!(
                        facts.iter().any(|fact| fact["kind"] == "deleted"),
                        known_no_effect,
                        "possible {name} recreation cannot retain absence (batch={batch})"
                    );
                }
            }
        }
        let omitted = observe(name, &json!({"path":"a.rs"}), Ok(&json!({})));
        let mut checkpoint = seeded();
        checkpoint.observe_serial(&delete);
        checkpoint.observe_serial(&omitted);
        assert!(
            !projection(&checkpoint)["coding_facts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|fact| fact["kind"] == "deleted")
        );
    }
}
