//! The root's working checkout W stays usable after a helper wrote it. Each
//! root-over-W verification must accept W in the lifecycle it is in, so the
//! direct-mode coordinator keeps working. Helpers share W, so the setup is
//! a helper that wrote W, not a merge — and because nothing integrates into W,
//! W stays `Active` rather than being demoted to `Retained`.

// The fixture composes child operations over a confined checkout; the Linux
// probe fails closed, so this suite needs a host that proves confinement.
#![cfg(target_os = "macos")]

use super::create::digest;
use super::direct_root_fixture::{
    agent, edit_root, fixture, helper_wrote_w, operations, root_checkout_id, root_invocation,
    root_path,
};
use saya_agent::ToolExecutor;
use saya_harness::workspace::Workspace;
use saya_types::{
    CodingResultOutcome, ManagedCheckoutId, ManagedCheckoutLifecycle, Sha256Digest, WorkspacePath,
};
use std::fs;

/// A helper that finished having written `path` in W.
async fn helper_wrote(job: &super::NativeCodingJob, name: &str, path: &str, bytes: &[u8]) {
    helper_wrote_w(
        job,
        name,
        vec![path.into()],
        CodingResultOutcome::Completed,
        "done",
        Some((path, bytes)),
    )
    .await;
}

#[tokio::test]
async fn resume_after_a_helper_wrote_w_recomposes_the_root_workspace() {
    let f = fixture("rr", vec!["tracked.txt".into(), "child-y.txt".into()]).await;
    edit_root(&f.job, "tracked.txt", b"root edited\n");
    helper_wrote(&f.job, "child-a", "child-y.txt", b"child bytes\n").await;
    // W is Retained: root_changes and the resume recompose both verify W.
    let changed = f.job.root_changes().await.unwrap();
    assert!(
        changed.iter().any(|change| change.path == "child-y.txt"),
        "the helper's file is among the root changes: {}",
        changed
            .iter()
            .map(|change| change.path.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let w = root_checkout_id(&f.job);
    f.job
        .compose_direct_workspace(&w, None)
        .await
        .expect("the retained root workspace recomposes");
}

#[tokio::test]
async fn the_root_can_still_write_edit_and_run_after_its_checkout_is_retained() {
    let f = fixture(
        "rw",
        vec![
            "tracked.txt".into(),
            "fresh.txt".into(),
            "edited.txt".into(),
        ],
    )
    .await;
    helper_wrote(&f.job, "child-a", "tracked.txt", b"child\n").await;
    let w = root_checkout_id(&f.job);
    let workspace = f
        .job
        .compose_direct_workspace(&w, None)
        .await
        .expect("the retained root workspace recomposes");
    workspace
        .tools
        .execute(
            "write",
            serde_json::json!({"path": "fresh.txt", "content": "root wrote"}),
        )
        .await
        .expect("write lands on the retained W");
    workspace
        .tools
        .execute(
            "edit",
            serde_json::json!({"path": "edited.txt", "offset": 0, "chunk": "root edited"}),
        )
        .await
        .expect("edit lands on the retained W");
    assert_eq!(
        fs::read_to_string(root_path(&f.job).join("fresh.txt")).unwrap(),
        "root wrote"
    );
    assert_eq!(
        fs::read_to_string(root_path(&f.job).join("edited.txt")).unwrap(),
        "root edited"
    );
}

#[tokio::test]
async fn workspace_delete_works_for_the_root_after_a_helper_wrote_w() {
    let f = fixture("rd", vec!["tracked.txt".into(), "child-y.txt".into()]).await;
    helper_wrote(&f.job, "child-a", "child-y.txt", b"child bytes\n").await;
    let w = root_checkout_id(&f.job);
    let token = f.job.cancellation();
    let digest = Sha256Digest::parse(
        &Workspace::open(&root_path(&f.job))
            .unwrap()
            .read("child-y.txt", 1024 * 1024)
            .unwrap()
            .digest,
    )
    .unwrap();
    let path = WorkspacePath::try_from("child-y.txt".to_string()).unwrap();
    let root_ops = operations(&f.job, &w, &agent("coordinator"), &root_invocation(&f.job)).await;
    root_ops
        .delete_workspace(path.clone(), digest, token.clone())
        .await
        .expect("the root deletes in W after a helper wrote it");
    assert!(
        !root_path(&f.job).join("child-y.txt").exists(),
        "the helper's file is removed from W"
    );
}

#[tokio::test]
async fn a_later_checkout_does_not_evict_the_root_working_checkout() {
    let f = fixture("re", vec!["tracked.txt".into(), "child-y.txt".into()]).await;
    helper_wrote(&f.job, "child-a", "child-y.txt", b"child\n").await;
    let w = root_checkout_id(&f.job);
    let candidate = ManagedCheckoutId::parse(&format!(
        "cand-{}",
        &digest(format!("{}:candidate", f.job.job_id().as_str()).as_bytes())
            .unwrap()
            .as_str()[..32]
    ))
    .unwrap();
    f.job
        .create_checkout(&candidate)
        .await
        .expect("a fresh candidate is created within the retention budget");
    let page = f
        .job
        .application
        .store
        .list_managed_checkouts(f.job.job_id(), None, 100)
        .await
        .unwrap();
    let w_record = page
        .items
        .iter()
        .find(|record| record.ownership.managed_checkout_id() == &w)
        .expect("W is not evicted");
    assert_eq!(
        w_record.current_lifecycle,
        ManagedCheckoutLifecycle::Active,
        "W stays the active working checkout when a later checkout is created"
    );
}
