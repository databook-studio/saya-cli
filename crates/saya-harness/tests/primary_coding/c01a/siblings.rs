use std::num::NonZeroU64;

use saya_harness::coding::commands::CommandOwner;

use super::support::{HELPER, fixture, process_is_alive, wait_for_file, wait_group_gone};

#[tokio::test]
async fn dropping_one_session_cancels_only_it_and_join_waits_for_cleanup() {
    let Some(fixture) = fixture() else {
        return;
    };
    let owner = CommandOwner::new(2).unwrap();
    let (parent_marker, child_marker) = fixture.marker_pair("sibling-stopped");
    let first = owner
        .start(
            fixture.run_program(),
            HELPER.into(),
            vec![
                "fork-hold".into(),
                parent_marker.display().to_string(),
                child_marker.display().to_string(),
            ],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let first_parent = wait_for_file(&parent_marker).await;
    let first_child = wait_for_file(&child_marker).await;

    let sibling_marker = fixture.marker("sibling-kept");
    let sibling = owner
        .start(
            fixture.run_program(),
            HELPER.into(),
            vec![
                "hold".into(),
                sibling_marker.display().to_string(),
                "30".into(),
            ],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let sibling_pid = wait_for_file(&sibling_marker).await;
    assert!(!sibling.is_finished());

    drop(first);
    assert!(wait_group_gone(first_parent).await);
    assert!(!process_is_alive(first_parent));
    assert!(!process_is_alive(first_child));
    assert!(process_is_alive(sibling_pid));
    assert!(!sibling.is_finished());

    let outcome = sibling.stop_and_join().await.unwrap();
    assert!(outcome.cancelled);
    assert!(wait_group_gone(outcome.pid).await);
}
