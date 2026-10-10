use std::{num::NonZeroU64, process::Command};

use saya_harness::coding::commands::CommandOwner;

use super::support::{HELPER, fixture, process_is_alive, wait_for_file, wait_group_gone};

struct UnrelatedProcess(std::process::Child);

impl Drop for UnrelatedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn owner_exit_cancels_forked_commands_and_leaves_unrelated_process_alone() {
    let Some(fixture) = fixture() else {
        return;
    };
    let owner = CommandOwner::new(2).unwrap();
    let unrelated = UnrelatedProcess(
        Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("unrelated test process starts"),
    );
    let (parent_marker, child_marker) = fixture.marker_pair("owner-exit");
    let session = owner
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
    let (second_parent_marker, second_child_marker) = fixture.marker_pair("owner-exit-second");
    let second = owner
        .start(
            fixture.run_program(),
            HELPER.into(),
            vec![
                "fork-hold".into(),
                second_parent_marker.display().to_string(),
                second_child_marker.display().to_string(),
            ],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let parent_pid = wait_for_file(&parent_marker).await;
    let child_pid = wait_for_file(&child_marker).await;
    let second_parent_pid = wait_for_file(&second_parent_marker).await;
    let second_child_pid = wait_for_file(&second_child_marker).await;
    assert!(process_is_alive(parent_pid));
    assert!(process_is_alive(child_pid));

    drop(owner);
    let outcome = session.join().await.unwrap();
    let second_outcome = second.join().await.unwrap();
    assert!(outcome.cancelled);
    assert!(second_outcome.cancelled);
    assert!(wait_group_gone(outcome.pid).await);
    assert!(wait_group_gone(second_outcome.pid).await);
    assert!(!process_is_alive(parent_pid));
    assert!(!process_is_alive(child_pid));
    assert!(!process_is_alive(second_parent_pid));
    assert!(!process_is_alive(second_child_pid));
    assert!(process_is_alive(unrelated.0.id()));
}
