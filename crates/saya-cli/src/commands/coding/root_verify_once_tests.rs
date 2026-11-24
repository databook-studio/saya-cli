//! The in-place root is verified **once per composition**.
//!
//! An in-place job's workspace composition opens the folder and reads its
//! published baseline to prove the root is the one the job recorded. That proof
//! is the composition's security check, and it is worth one read: the layer
//! that verified the root binds the value it got and hands it on, instead of
//! passing "not verified" down to a second read of the same folder and the same
//! baseline (measured: ~half of the 478 ms the in-place toolset's composition
//! cost, on a folder inside a repository).
//!
//! The count comes from the harness's census
//! (`saya_harness::coding::worktree::in_place_verifications`), which is keyed
//! by the root's own path: this test reads the count for the folder it made, so
//! nothing else in the same process can be mistaken for it.

use std::path::Path;

use saya_harness::coding::worktree::in_place_verifications;

use super::rewind::in_place_fixture::fixture;

/// One composition, one verification: the root the composition proves is the
/// root its toolset is built over, and it is not proved a second time.
#[tokio::test]
async fn composing_the_in_place_workspace_verifies_the_root_once() {
    let fixture = fixture("verify-once", vec!["tracked.txt".into()]).await;
    let record = fixture
        .job
        .in_place_record()
        .expect("an in-place job records the root it works over");
    let root = Path::new(record.root_path()).to_path_buf();
    let before = in_place_verifications(&root);
    let workspace = fixture
        .job
        .compose_in_place_workspace()
        .await
        .expect("the in-place workspace composes over the job's own folder");
    let after = in_place_verifications(&root);
    assert_eq!(
        after - before,
        1,
        "one composition verifies the in-place root once ({before} before, {after} after): \
         the composition binds the proof it made instead of letting the layer below re-read \
         the folder and its baseline"
    );
    // The toolset it built is over that same root: the proof was handed on, not
    // discarded after it was made.
    assert!(
        workspace.definitions.iter().any(|tool| tool.name == "read"),
        "the composed toolset is the workspace's, built over the verified root"
    );
}
