//! The model-facing `rewind` tool over the job's own checkpoint store: it is
//! advertised from the run's first request (with an empty listing before the
//! first checkpoint), lists the checkpoints it holds, restores one path or
//! every path the run changed, and reports the digests a following `edit`
//! needs. A closed plan posture hides it, and a call it does not advertise is
//! refused as an unknown tool.

use std::path::{Path, PathBuf};

use saya_agent::{ApprovalDecider, LocalStateEffect, ToolError, ToolExecutor};
use serde_json::{Value, json};

use crate::agent::tools::coding::RootTools;
use crate::commands::coding::NativeCodingJob;
use crate::commands::coding::root_checkout_tests::{fixture, plan_fixture};

async fn checkout(job: &NativeCodingJob) -> PathBuf {
    PathBuf::from(
        job.verified_root_checkout()
            .await
            .unwrap()
            .expect("the fixture job has a working checkout")
            .path()
            .as_str(),
    )
}

fn write(path: &Path, name: &str, content: &str) {
    std::fs::write(path.join(name), content).unwrap();
}

fn read(path: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(path.join(name)).ok()
}

fn advertised(root: &RootTools, name: &str) -> bool {
    root.definitions().iter().any(|tool| tool.name == name)
}

/// The first tool call whose result is an object with `key`, or `Value::Null`.
fn entry<'a>(value: &'a Value, key: &str) -> &'a Value {
    value[key].get(0).unwrap_or(&Value::Null)
}

/// One checkpoint holding `tracked.txt`, then a second holding that file
/// changed plus one created after the first checkpoint.
async fn two_checkpoints(job: &NativeCodingJob) -> PathBuf {
    job.ensure_root_checkout().await.unwrap().unwrap();
    let checkout = checkout(job).await;
    write(&checkout, "tracked.txt", "one\n");
    job.capture_rewind().await;
    write(&checkout, "tracked.txt", "changed\n");
    write(&checkout, "later.txt", "later\n");
    job.capture_rewind().await;
    checkout
}

#[cfg(unix)]
#[tokio::test]
async fn the_model_lists_restores_one_path_and_then_all_of_them() {
    let fixture = fixture(
        "model-rewind",
        vec!["tracked.txt".into(), "later.txt".into()],
    )
    .await;
    let checkout = two_checkpoints(&fixture.job).await;
    let root = RootTools::build(&fixture.job).await.unwrap().0;

    let definition = root
        .definitions()
        .into_iter()
        .find(|tool| tool.name == "rewind")
        .expect("the run has a checkpoint, so rewind is advertised");
    assert_eq!(
        definition.effect.local_state,
        LocalStateEffect::WriteWorkspace
    );
    assert!(definition.effect.requires_approval);
    assert!(root.approve(&definition, &json!({})).await);

    let listed = root.execute("rewind", json!({})).await.unwrap();
    let checkpoints = listed["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 2, "{listed}");
    assert_eq!(checkpoints[0]["id"], "2");
    assert_eq!(checkpoints[1]["id"], "1");
    assert_eq!(listed["maximum_checkpoints"], 32);
    assert!(
        checkpoints[0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "later.txt"),
        "{listed}"
    );
    assert!(
        checkpoints[1]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| path != "later.txt"),
        "the first checkpoint predates the second file: {listed}"
    );

    // `last` is the newest checkpoint: the state the checkout already holds.
    let last = root
        .execute("rewind", json!({"checkpoint": "last"}))
        .await
        .unwrap();
    assert_eq!(last["checkpoint"], 2);
    assert_eq!(read(&checkout, "tracked.txt").as_deref(), Some("changed\n"));

    // Restoring one path puts that file back and leaves the other changed
    // path exactly as it was.
    let one = root
        .execute(
            "rewind",
            json!({"checkpoint": "1", "paths": ["tracked.txt"]}),
        )
        .await
        .unwrap();
    let restored = entry(&one, "restored_paths");
    assert_eq!(restored["path"], "tracked.txt");
    assert_eq!(read(&checkout, "tracked.txt").as_deref(), Some("one\n"));
    assert_eq!(
        read(&checkout, "later.txt").as_deref(),
        Some("later\n"),
        "a path outside `paths` is never touched"
    );
    assert_eq!(one["removed_paths"].as_array().unwrap().len(), 0);

    // The digest the restore reports is the one a following read observes, so
    // an `edit` can use it without another read.
    let observed = root
        .execute("read", json!({"path": "tracked.txt"}))
        .await
        .unwrap();
    assert_eq!(restored["sha256"], observed["digest"]);
    assert_eq!(restored["size"], observed["size"]);

    // Restoring every path the run changed since the checkpoint removes the
    // file that did not exist then.
    let all = root
        .execute("rewind", json!({"checkpoint": "1"}))
        .await
        .unwrap();
    assert_eq!(all["checkpoint"], 1);
    assert_eq!(
        all["restored_paths"],
        json!([{
            "path": "tracked.txt",
            "sha256": restored["sha256"],
            "size": restored["size"],
        }])
    );
    assert_eq!(all["removed_paths"], json!(["later.txt"]));
    assert_eq!(read(&checkout, "tracked.txt").as_deref(), Some("one\n"));
    assert_eq!(read(&checkout, "later.txt"), None);
}

#[cfg(unix)]
#[tokio::test]
async fn rewind_is_advertised_from_the_first_request_with_an_empty_listing() {
    let fixture = fixture("model-rewind-evidence", vec!["tracked.txt".into()]).await;
    fixture.job.ensure_root_checkout().await.unwrap().unwrap();
    let checkout = checkout(&fixture.job).await;
    let root = RootTools::build(&fixture.job).await.unwrap().0;
    assert!(
        advertised(&root, "rewind"),
        "the tool is offered from the first request, before any checkpoint exists"
    );
    let listed = root.execute("rewind", json!({})).await.unwrap();
    assert_eq!(listed["checkpoints"], json!([]));
    assert_eq!(listed["maximum_checkpoints"], 32);

    write(&checkout, "tracked.txt", "one\n");
    fixture.job.capture_rewind().await;
    let root = RootTools::build(&fixture.job).await.unwrap().0;
    assert!(
        advertised(&root, "rewind"),
        "the first checkpoint does not change the advertised list"
    );
    let listed = root.execute("rewind", json!({})).await.unwrap();
    assert_eq!(listed["checkpoints"].as_array().unwrap().len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn a_closed_plan_posture_hides_rewind_until_the_operator_opens_it() {
    let fixture = plan_fixture("model-rewind-plan", vec!["tracked.txt".into()]).await;
    let checkout = two_checkpoints(&fixture.job).await;
    let root = RootTools::build(&fixture.job).await.unwrap().0;
    assert!(!advertised(&root, "rewind"));
    assert!(matches!(
        root.execute("rewind", json!({})).await,
        Err(ToolError::UnsupportedTool)
    ));

    fixture.job.plan_gate().unwrap().cell().open();
    assert!(advertised(&root, "rewind"));
    let listed = root.execute("rewind", json!({})).await.unwrap();
    assert_eq!(listed["checkpoints"].as_array().unwrap().len(), 2);
    assert_eq!(read(&checkout, "tracked.txt").as_deref(), Some("changed\n"));
}

#[cfg(unix)]
#[tokio::test]
async fn an_unknown_checkpoint_is_refused_by_name_and_changes_nothing() {
    let fixture = fixture(
        "model-rewind-unknown",
        vec!["tracked.txt".into(), "later.txt".into()],
    )
    .await;
    let checkout = two_checkpoints(&fixture.job).await;
    let root = RootTools::build(&fixture.job).await.unwrap().0;

    assert!(matches!(
        root.execute("rewind", json!({"checkpoint": "9"})).await,
        Err(ToolError::NoEffect(_))
    ));
    assert!(matches!(
        root.execute("rewind", json!({"checkpoint": "not-an-id"}))
            .await,
        Err(ToolError::NoEffect(_))
    ));
    assert_eq!(read(&checkout, "tracked.txt").as_deref(), Some("changed\n"));
    assert_eq!(read(&checkout, "later.txt").as_deref(), Some("later\n"));
}
