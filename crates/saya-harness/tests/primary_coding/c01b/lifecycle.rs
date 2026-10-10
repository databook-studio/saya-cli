use std::num::NonZeroU64;

use saya_agent::CancellationToken;
use saya_harness::coding::commands::{CommandInputError, CommandOwner, CommandProcessState};

use super::wait_for_state;
use crate::support::{fixture, process_is_alive, wait_for_file};

const CHUNK_BYTES: usize = 8 * 1024;

#[tokio::test(flavor = "current_thread")]
async fn queue_full_refuses_before_dispatch_and_unread_input_stop_preserves_sibling() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(2).unwrap();
    let never_started = fixture.marker("queued-only-pid");
    let queued = owner
        .start_with_stdin(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["hold".into(), never_started.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    assert_eq!(queued.process_state(), CommandProcessState::Pending);
    let chunk = vec![b'q'; CHUNK_BYTES];
    for _ in 0..16 {
        assert_eq!(queued.send_stdin(&chunk).unwrap().bytes(), CHUNK_BYTES);
    }
    assert!(matches!(
        queued.send_stdin(&chunk),
        Err(CommandInputError::QueueFull { max_chunks: 16 })
    ));
    assert!(!never_started.exists());
    queued.stop();
    wait_for_state(&queued, CommandProcessState::NotSpawned).await;
    assert!(matches!(
        queued.join().await,
        Err(saya_harness::coding::commands::CommandSessionError::CancelledBeforeStart)
    ));
    assert!(!never_started.exists());

    let first_marker = fixture.marker("unread-input-pid");
    let sibling_marker = fixture.marker("sibling-pid");
    let first = owner
        .start_with_stdin(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["hold".into(), first_marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let sibling = owner
        .start(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["hold".into(), sibling_marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let first_pid = wait_for_file(&first_marker).await;
    let sibling_pid = wait_for_file(&sibling_marker).await;
    assert!(matches!(
        first.send_stdin(&chunk),
        Ok(receipt) if receipt.bytes() == CHUNK_BYTES
    ));

    first.stop();
    wait_for_state(&first, CommandProcessState::Reaped { pid: first_pid }).await;
    let first_outcome = first.join().await.unwrap();
    assert!(first_outcome.cancelled);
    assert_eq!(first_outcome.pid, first_pid);
    assert!(!process_is_alive(first_pid));
    assert!(process_is_alive(sibling_pid));

    let third_marker = fixture.marker("released-capacity-pid");
    let third = owner
        .start(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["hold".into(), third_marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .expect("joined input worker releases the owned command slot");
    let third_pid = wait_for_file(&third_marker).await;
    assert!(process_is_alive(sibling_pid));
    assert!(third.stop_and_join().await.unwrap().cancelled);
    assert!(!process_is_alive(third_pid));
    assert!(sibling.stop_and_join().await.unwrap().cancelled);
    assert!(!process_is_alive(sibling_pid));

    let owner_stopped = CommandOwner::new(1).unwrap();
    let owner_marker = fixture.marker("owner-stop-pid");
    let owner_session = owner_stopped
        .start_with_stdin(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["hold".into(), owner_marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let owner_pid = wait_for_file(&owner_marker).await;
    assert!(owner_session.send_stdin(&chunk).is_ok());
    owner_stopped.cancel_all();
    wait_for_state(
        &owner_session,
        CommandProcessState::Reaped { pid: owner_pid },
    )
    .await;
    assert!(owner_session.join().await.unwrap().cancelled);
    assert!(!process_is_alive(owner_pid));
}

#[tokio::test]
async fn caller_run_cancellation_stops_and_reaps_spawned_command() {
    let Some(fixture) = fixture() else { return };
    let cancellation = CancellationToken::new();
    let marker = fixture.marker("caller-cancel-pid");
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_stdin(
            fixture
                .run_program()
                .with_cancellation(cancellation.clone()),
            crate::support::HELPER.to_owned(),
            vec!["hold".into(), marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let pid = wait_for_file(&marker).await;

    cancellation.cancel();
    wait_for_state(&session, CommandProcessState::Reaped { pid }).await;
    let outcome = session.join().await.unwrap();
    assert!(outcome.cancelled);
    assert_eq!(outcome.pid, pid);
    assert!(!process_is_alive(pid));
}

#[tokio::test]
async fn owner_cancellation_immediately_refuses_new_stdin() {
    let Some(fixture) = fixture() else { return };
    let marker = fixture.marker("owner-cancel-input-pid");
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_stdin(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["hold".into(), marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let pid = wait_for_file(&marker).await;

    owner.cancel_all();
    assert!(matches!(
        session.send_stdin(b"must be refused"),
        Err(CommandInputError::Closed)
    ));
    wait_for_state(&session, CommandProcessState::Reaped { pid }).await;
    assert!(session.join().await.unwrap().cancelled);
    assert!(!process_is_alive(pid));
}
