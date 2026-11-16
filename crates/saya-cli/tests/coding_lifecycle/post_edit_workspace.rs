#[path = "post_edit_workspace_support.rs"]
mod support;
use support::*;

#[path = "post_edit_workspace_failures.rs"]
mod failures;
use super::provider::Provider;
use std::fs;

#[tokio::test]
async fn automatic_post_edit_default_off_adds_no_feedback_or_extension_receipt() {
    let run = run(
        "post-edit-off",
        Provider::post_edit(),
        30,
        false,
        &lsp_server(false, false, false),
    )
    .await;
    let results = tool_results(&run.provider);
    assert!(run.outcome.is_ok());
    assert!(run.checkout.join("new.txt").is_file());
    assert!(results["call-write"].get("post_edit_diagnostics").is_none());
    assert!(results["call-edit"].get("post_edit_diagnostics").is_none());
    assert!(
        results["post-write-replace"]
            .get("post_edit_diagnostics")
            .is_none()
    );
    assert_eq!(
        fs::read(run.checkout.join("new.txt")).unwrap(),
        b"created and edited\n"
    );
    assert!(!record_path(&run, "known-commits-").exists());
    assert!(!record_path(&run, "extensions-").exists());
}

// Post-edit feedback spawns a confined LSP extension process, which only
// macOS proves. Linux fails closed before any process starts; the Linux
// counterpart at the end of this module pins that refusal.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn native_multi_edit_uses_committed_bytes_for_confined_lsp_feedback_and_export() {
    let run = run(
        "post-edit-complete",
        Provider::post_edit(),
        30,
        true,
        &lsp_server(false, false, false),
    )
    .await;
    let results = tool_results(&run.provider);
    let write = &results["call-write"];
    let edit = &results["call-edit"];
    let replace = &results["post-write-replace"];
    let written = fs::read(run.checkout.join("new.txt")).unwrap();
    let edited = fs::read(run.checkout.join("tracked.txt")).unwrap();

    assert!(run.outcome.is_ok());
    assert_eq!(written, b"created and edited\n");
    assert_eq!(edited, b"updated\n");
    for result in [write, edit, replace] {
        assert_eq!(result["post_edit_diagnostics"]["status"], "complete");
        assert_eq!(result["post_edit_diagnostics"]["version"], 1);
        assert_eq!(
            result["post_edit_diagnostics"]["document_sha256"],
            result["digest"]
        );
        assert_eq!(
            result["post_edit_diagnostics"]["diagnostics"][0]["message"],
            "fixture diagnostic"
        );
        assert_eq!(result["post_edit_diagnostics"]["edits_applied"], false);
    }
    let original_write = b"created by write\n";
    for (result, bytes) in [
        (write, original_write.as_slice()),
        (edit, edited.as_slice()),
        (replace, written.as_slice()),
    ] {
        assert_eq!(result["digest"], sha256_hex(bytes));
        assert_eq!(result["size"].as_u64(), Some(bytes.len() as u64));
    }
    assert_eq!(write["path"], "new.txt");
    assert_eq!(edit["path"], "tracked.txt");
    assert_eq!(replace["path"], "new.txt");
    let commits = jsonl(&record_path(&run, "known-commits-"));
    assert_eq!(commits.len(), 3);
    assert!(commits.iter().any(|record| record["operation"] == "write"));
    assert_eq!(
        commits
            .iter()
            .filter(|record| record["operation"] == "edit")
            .count(),
        2
    );
    for (result, operation) in [(write, "write"), (edit, "edit"), (replace, "edit")] {
        let committed = commits
            .iter()
            .find(|record| {
                record["operation"] == operation
                    && record["path"] == result["path"]
                    && record["committed_sha256"] == result["digest"]
                    && record["size"] == result["size"]
            })
            .expect("the strict receipt carries the exact committed result facts");
        assert_eq!(committed["checkout_id"], run.checkout_id);
    }
    let extension_receipts = jsonl(&record_path(&run, "extensions-"));
    assert_eq!(extension_receipts.len(), 6);
    for pair in extension_receipts.as_chunks::<2>().0 {
        assert_eq!(pair[0]["fact"]["phase"], "spawned");
        assert!(pair[0]["fact"].get("joined").is_none());
        assert_eq!(pair[1]["fact"]["phase"], "settled");
        assert_eq!(pair[1]["fact"]["joined"], true);
    }

    let exported = run
        .fixture
        .application
        .export(&run.job_id, None, None)
        .await
        .unwrap();
    let files = exported["artifacts"]["files"].as_array().unwrap();
    let safe_commits = files
        .iter()
        .find(|file| file["name"].as_str().unwrap().starts_with("known-commits-"))
        .unwrap()["records"]
        .as_array()
        .unwrap();
    assert_eq!(safe_commits.len(), 3);
    assert!(safe_commits.iter().all(|record| {
        record["path"]["projection"] == "private_relative_path_sha256"
            && record["path"]["sha256"].as_str().is_some()
    }));
    for (result, operation) in [(write, "write"), (edit, "edit"), (replace, "edit")] {
        let private_record = commits
            .iter()
            .find(|record| {
                record["operation"] == operation
                    && record["path"] == result["path"]
                    && record["committed_sha256"] == result["digest"]
                    && record["size"] == result["size"]
            })
            .expect("the strict receipt carries each actual committed mutation");
        let projected = safe_commits
            .iter()
            .find(|record| record["committed_sha256"] == result["digest"])
            .expect("safe export keeps each known commit");
        assert_eq!(projected["path"]["sha256"], private_record["path_sha256"]);
    }
    let safe_json = serde_json::to_string(safe_commits).unwrap();
    assert!(!safe_json.contains("tracked.txt"));
    assert!(!safe_json.contains("new.txt"));
}

/// The extension gate is the trusted profile and its `program_dir`
/// confinement, never a per-run approved program list: a run that approved no
/// program still runs the extension, and
/// every write's post-edit exchange spawns and settles.
// Post-edit feedback spawns a confined LSP extension process, which only
// macOS proves. Linux fails closed before any process starts; the Linux
// counterpart at the end of this module pins that refusal.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_extension_runs_without_an_approved_program() {
    let run = run(
        "post-edit-no-runner",
        Provider::post_edit_without_runner(),
        32,
        true,
        &lsp_server(false, false, false),
    )
    .await;
    let results = tool_results(&run.provider);
    assert!(run.outcome.is_ok(), "outcome: {:?}", run.outcome);
    assert_eq!(
        fs::read(run.checkout.join("new.txt")).unwrap(),
        b"committed despite refusal\n"
    );
    assert!(run.checkout.join("followup.txt").is_file());
    assert_eq!(jsonl(&record_path(&run, "known-commits-")).len(), 2);
    let extension_receipts = jsonl(&record_path(&run, "extensions-"));
    assert_eq!(extension_receipts.len(), 4);
    for pair in extension_receipts.as_chunks::<2>().0 {
        assert_eq!(pair[0]["fact"]["phase"], "spawned");
        assert_eq!(pair[1]["fact"]["phase"], "settled");
        assert_eq!(pair[1]["fact"]["joined"], true);
    }
    assert_eq!(
        results["call-write"]["post_edit_diagnostics"]["status"],
        "complete"
    );
}

// Post-edit feedback spawns a confined LSP extension process, which only
// macOS proves. Linux fails closed before any process starts; the Linux
// counterpart at the end of this module pins that refusal.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn stale_after_exchange_invalidates_the_implicit_witness() {
    let run = failures::run_stale_after_exchange().await;
    let results = tool_results(&run.provider);
    let terminal = run
        .fixture
        .store
        .latest_coding_run_event(&run.job_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        run.outcome.is_ok(),
        "the fixture restores its external mutation after capturing the stale refusal; outcome={:?}; terminal={:?}; write={:?}; edit={:?}",
        run.outcome,
        terminal.event(),
        results.get("call-write"),
        results.get("call-edit")
    );
    assert!(matches!(
        terminal.event(),
        saya_types::CodingEventKind::StateChanged {
            state: saya_types::JobState::Completed
        }
    ));
    assert_eq!(
        results["call-write"]["post_edit_diagnostics"]["status"],
        "stale"
    );
    assert!(
        results["call-edit"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("expected_digest"))
    );
    assert_eq!(
        fs::read(run.checkout.join("new.txt")).unwrap(),
        b"initial\n"
    );
}

// Post-edit feedback spawns a confined LSP extension process, which only
// macOS proves. Linux fails closed before any process starts; the Linux
// counterpart at the end of this module pins that refusal.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn unavailable_feedback_follows_a_joined_observer_receipt_and_invalidates_witness() {
    let run = run(
        "post-edit-unavailable",
        Provider::post_edit_unavailable(),
        33,
        true,
        &lsp_server(false, true, false),
    )
    .await;
    let results = tool_results(&run.provider);
    let extension_receipts = jsonl(&record_path(&run, "extensions-"));
    let terminal = run
        .fixture
        .store
        .latest_coding_run_event(&run.job_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        run.outcome.is_ok(),
        "unavailable diagnostics must settle non-fatally after observer join; outcome={:?}; terminal={:?}; write={:?}; edit={:?}; extension_receipts={:?}",
        run.outcome,
        terminal.event(),
        results.get("call-write"),
        results.get("call-edit"),
        extension_receipts
    );
    assert!(matches!(
        terminal.event(),
        saya_types::CodingEventKind::StateChanged {
            state: saya_types::JobState::Completed
        }
    ));
    assert_eq!(
        results["call-write"]["post_edit_diagnostics"]["status"],
        "unavailable"
    );
    assert_eq!(extension_receipts.len(), 2);
    assert_eq!(extension_receipts[0]["fact"]["phase"], "spawned");
    assert_eq!(extension_receipts[1]["fact"]["phase"], "settled");
    assert_eq!(extension_receipts[1]["fact"]["joined"], true);
    assert!(
        results["call-edit"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("expected_digest"))
    );
}

// The Linux counterpart: without proven confinement no extension process may
// start, so post-edit feedback fails the run closed. The write it was
// reporting on is already committed and stays in the checkout.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_confined_extension_cannot_start_without_proven_confinement() {
    let run = run(
        "post-edit-no-confinement",
        Provider::post_edit(),
        30,
        true,
        &lsp_server(false, false, false),
    )
    .await;
    assert!(
        run.outcome.is_err(),
        "the run fails closed: {:?}",
        run.outcome
    );
    assert!(
        !record_path(&run, "extensions-").exists(),
        "no extension process started, so no receipt exists"
    );
    assert_eq!(
        fs::read(run.checkout.join("new.txt")).unwrap(),
        b"created by write\n",
        "the committed write survives the refused exchange"
    );
}
