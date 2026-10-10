use std::{num::NonZeroU64, time::Duration};

use saya_harness::coding::commands::{
    CommandInputError, CommandOwner, CommandProcessState, MAX_COMMAND_INPUT_BYTES,
    MAX_COMMAND_INPUT_CHUNK_BYTES,
};

use super::wait_for_state;
use crate::support::fixture;

#[tokio::test]
async fn post_spawn_input_is_queued_then_closed_and_reports_real_lifecycle() {
    let Some(fixture) = fixture() else { return };
    let marker = fixture.marker("echo-pid");
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_stdin(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["echo".into(), marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    assert_eq!(session.process_state(), CommandProcessState::Pending);

    let pid = crate::support::wait_for_file(&marker).await;
    assert_eq!(
        session.process_state(),
        CommandProcessState::Spawned { pid }
    );
    let bytes = b"post-spawn bytes, followed by explicit EOF";
    let queued = session.send_stdin(bytes).unwrap();
    assert_eq!(queued.bytes(), bytes.len());
    session.close_stdin().unwrap();
    assert!(matches!(
        session.send_stdin(b"late"),
        Err(CommandInputError::Closed)
    ));
    assert!(matches!(
        session.close_stdin(),
        Err(CommandInputError::Closed)
    ));

    wait_for_state(&session, CommandProcessState::Reaped { pid }).await;
    let outcome = session.join().await.unwrap();
    assert_eq!(outcome.pid, pid);
    assert_eq!(outcome.exit_code, Some(0));
    let expected_sum: u64 = bytes.iter().map(|byte| u64::from(*byte)).sum();
    assert_eq!(
        outcome.stdout.text.trim(),
        format!("{}:{expected_sum}", bytes.len())
    );
}

#[tokio::test]
async fn exact_lifetime_input_cap_is_accepted_and_one_more_byte_is_refused() {
    let Some(fixture) = fixture() else { return };
    let marker = fixture.marker("cap-pid");
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_stdin(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["echo".into(), marker.to_str().unwrap().into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    let pid = crate::support::wait_for_file(&marker).await;
    let chunk = vec![b'x'; MAX_COMMAND_INPUT_CHUNK_BYTES];
    let oversized = vec![b'y'; MAX_COMMAND_INPUT_CHUNK_BYTES + 1];
    assert!(matches!(
        session.send_stdin(&oversized),
        Err(CommandInputError::ChunkTooLarge {
            max_bytes: MAX_COMMAND_INPUT_CHUNK_BYTES
        })
    ));
    let accepted = tokio::time::timeout(Duration::from_secs(10), async {
        let mut accepted = 0;
        while accepted < MAX_COMMAND_INPUT_BYTES {
            match session.send_stdin(&chunk) {
                Ok(receipt) => accepted += receipt.bytes(),
                Err(CommandInputError::QueueFull { .. }) => {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Err(error) => panic!("unexpected stdin refusal at {accepted} bytes: {error}"),
            }
        }
        accepted
    })
    .await
    .expect("the helper drains accepted input before the finite deadline");
    assert_eq!(accepted, MAX_COMMAND_INPUT_BYTES);
    assert!(matches!(
        session.send_stdin(b"x"),
        Err(CommandInputError::LifetimeLimitExceeded {
            max_bytes: MAX_COMMAND_INPUT_BYTES
        })
    ));
    session.close_stdin().unwrap();

    wait_for_state(&session, CommandProcessState::Reaped { pid }).await;
    let outcome = session.join().await.unwrap();
    assert_eq!(outcome.pid, pid);
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(
        outcome.stdout.text.trim(),
        format!(
            "{MAX_COMMAND_INPUT_BYTES}:{}",
            MAX_COMMAND_INPUT_BYTES as u64 * u64::from(b'x')
        )
    );
}
