use std::num::NonZeroU64;

use saya_harness::coding::commands::{CommandOwner, CommandOwnerError, CommandStartError};

use super::support::{HELPER, fixture, wait_for_file, wait_group_gone};

#[tokio::test]
async fn capacity_stays_reserved_until_the_owned_runner_finishes_cleanup() {
    let Some(fixture) = fixture() else {
        return;
    };
    assert!(matches!(
        CommandOwner::new(3),
        Err(CommandOwnerError::InvalidConcurrency)
    ));
    let owner = CommandOwner::new(1).unwrap();
    let (parent_marker, child_marker) = fixture.marker_pair("capacity-first");
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
    wait_for_file(&child_marker).await;

    let second_marker = fixture.marker("capacity-second");
    let start_second = || {
        owner.start(
            fixture.run_program(),
            HELPER.into(),
            vec![
                "hold".into(),
                second_marker.display().to_string(),
                "30".into(),
            ],
            NonZeroU64::new(20).unwrap(),
        )
    };
    assert!(matches!(start_second(), Err(CommandStartError::AtCapacity)));
    assert!(!second_marker.exists(), "denial precedes command dispatch");

    drop(first);
    assert!(matches!(start_second(), Err(CommandStartError::AtCapacity)));
    assert!(
        !second_marker.exists(),
        "a dropped handle cannot release its permit"
    );
    assert!(wait_group_gone(first_parent).await);

    let second = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match start_second() {
                Ok(session) => break session,
                Err(CommandStartError::AtCapacity) => {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                Err(error) => panic!("the finite command owner remains valid: {error}"),
            }
        }
    })
    .await
    .expect("cleanup releases the command slot");
    let second_pid = wait_for_file(&second_marker).await;
    assert!(super::support::process_is_alive(second_pid));
    let outcome = second.stop_and_join().await.unwrap();
    assert!(outcome.cancelled);
}
