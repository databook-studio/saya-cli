use std::{num::NonZeroU64, time::Duration};

use saya_harness::coding::commands::{
    CommandInputError, CommandOwner, CommandProcessState, CommandSession, CommandStartError,
    MAX_COMMAND_INPUT_CHUNK_BYTES, MAX_COMMAND_INPUT_QUEUE_CHUNKS,
};

use super::wait_for_state;
use crate::support::{fixture, process_is_alive, wait_for_file};

/// QueueFull refusals required back-to-back, with no successful send between
/// them, before the queue counts as saturated. One refusal can race with the
/// writer thread, which may still pop a queued chunk and admit one more send;
/// a sustained streak can only persist while the writer is stuck on a chunk
/// it cannot finish writing.
const SUSTAINED_QUEUE_FULLS: usize = 3;

/// Drives stdin sends until QueueFull is refused `SUSTAINED_QUEUE_FULLS`
/// times in a row with no successful send in between, within a 10 s deadline.
/// Returns the number of chunks admitted. What is observed is the refusal
/// streak alone: the helper sleeps after every attempt, including the third
/// refusal, so by the time it returns — and at any later stop or drop — the
/// writer may already have popped a queued chunk. Callers must not infer
/// that the queue holds its cap at any moment after the streak.
///
/// Every successful receipt counts one full chunk entering the bounded
/// session queue. These counts are queue-admission accounting only: a
/// receipt does not record that the runner wrote the chunk to the child's
/// pipe, and the public session result suppresses stdin delivery accounting
/// for cancelled outcomes (src/coding/commands/session_errors.rs:40-42), so
/// no test through this API can prove delivery counts.
///
/// Sustained backpressure is reachable with the non-reading `hold` helper: it
/// never reads stdin (it records its pid and sleeps), so the writer thread
/// can move queued chunks only into the kernel pipe, whose capacity absorbs a
/// bounded number of 8 KiB chunks. Once the pipe is full the writer holds one
/// chunk in flight that cannot complete, so sends keep being refused; the
/// observed streak establishes that those refusals held with no intervening
/// success over the helper's window. Total committed bytes stay far below
/// the 1 MiB stdin lifetime limit, so the streak cannot hit
/// LifetimeLimitExceeded first.
async fn saturate_stdin_queue(session: &CommandSession, chunk: &[u8]) -> usize {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut accepted_chunks = 0usize;
        let mut accepted_bytes = 0usize;
        let mut queue_full_streak = 0usize;
        while queue_full_streak < SUSTAINED_QUEUE_FULLS {
            match session.send_stdin(chunk) {
                Ok(receipt) => {
                    queue_full_streak = 0;
                    accepted_chunks += 1;
                    accepted_bytes += receipt.bytes();
                }
                Err(CommandInputError::QueueFull { max_chunks }) => {
                    assert_eq!(
                        max_chunks, MAX_COMMAND_INPUT_QUEUE_CHUNKS,
                        "QueueFull must name the bounded queue's chunk cap"
                    );
                    queue_full_streak += 1;
                }
                Err(error) => panic!(
                    "unexpected stdin refusal after {accepted_chunks} queued chunks: {error}"
                ),
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        // Admission accounting: the sum of successful receipts equals the
        // admitted chunk count times the fixed chunk size. This proves queue
        // admission only, not delivery into the child's pipe.
        assert_eq!(
            accepted_bytes,
            accepted_chunks * MAX_COMMAND_INPUT_CHUNK_BYTES
        );
        accepted_chunks
    })
    .await
    .expect("sustained QueueFull streak observed within the 10 s deadline")
}

/// Asserts stdin queue-admission accounting and terminal refusals, then
/// observes that stop settles the spawned session. The precondition is the
/// observed streak alone: three consecutive QueueFull refusals with no
/// intervening success, followed by stop. Because the helper sleeps after
/// the third refusal, the writer may have popped a queued chunk before stop
/// runs, so nothing here claims the queue holds its cap at the stop itself.
/// Nothing here proves how many queued bytes reached the child's pipe; see
/// `saturate_stdin_queue`.
#[tokio::test]
async fn stop_settles_spawned_session_after_consecutive_queue_full_refusals() {
    let Some(fixture) = fixture() else { return };
    let marker = fixture.marker("full-queue-stop-pid");
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
    assert!(matches!(
        session.process_state(),
        CommandProcessState::Spawned { .. }
    ));

    let chunk = vec![b'q'; MAX_COMMAND_INPUT_CHUNK_BYTES];
    let accepted_chunks = saturate_stdin_queue(&session, &chunk).await;
    // At the first refusal the queue holds exactly its cap of pending chunks,
    // all previously admitted; earlier admissions may already have been
    // popped toward the child's pipe or sit in the writer's in-flight chunk.
    assert!(accepted_chunks >= MAX_COMMAND_INPUT_QUEUE_CHUNKS);

    session.stop();
    assert!(matches!(
        session.send_stdin(&chunk),
        Err(CommandInputError::Closed)
    ));
    wait_for_state(&session, CommandProcessState::Reaped { pid }).await;
    let outcome = tokio::time::timeout(Duration::from_secs(10), session.join())
        .await
        .expect("join after stop: the cancelled spawned session settles within 10 s")
        .unwrap();
    assert!(outcome.cancelled);
    assert_eq!(outcome.pid, pid);
    assert!(!process_is_alive(pid));
}

/// Dropping after stdin sends have shown the sustained QueueFull streak
/// (three consecutive refusals with no intervening success — an observed
/// streak, not a claim that the queue holds its cap at the drop) still
/// releases the owned slot once cleanup finishes. Which queued chunks
/// reached the child before the drop is not observable from the public API;
/// see `saturate_stdin_queue`.
#[tokio::test]
async fn dropping_spawned_session_after_consecutive_queue_full_refusals_releases_owner_slot() {
    let Some(fixture) = fixture() else { return };
    let marker = fixture.marker("dropped-session-pid");
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
    assert!(matches!(
        session.process_state(),
        CommandProcessState::Spawned { .. }
    ));
    let chunk = vec![b'q'; MAX_COMMAND_INPUT_CHUNK_BYTES];
    saturate_stdin_queue(&session, &chunk).await;

    drop(session);
    wait_for_child_exit(pid).await;

    let successor_marker = fixture.marker("released-slot-pid");
    let successor = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match owner.start(
                fixture.run_program(),
                crate::support::HELPER.to_owned(),
                vec!["hold".into(), successor_marker.to_str().unwrap().into()],
                NonZeroU64::new(20).unwrap(),
            ) {
                Ok(session) => break session,
                Err(CommandStartError::AtCapacity) => {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                Err(error) => panic!("unexpected admission refusal: {error}"),
            }
        }
    })
    .await
    .expect("dropping the session releases the owned command slot");
    let successor_pid = wait_for_file(&successor_marker).await;
    let successor_outcome =
        tokio::time::timeout(Duration::from_secs(10), successor.stop_and_join())
            .await
            .expect("successor stop_and_join: the successor session settles within 10 s")
            .unwrap();
    assert!(successor_outcome.cancelled);
    assert!(!process_is_alive(successor_pid));
}

async fn wait_for_child_exit(pid: u32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while process_is_alive(pid) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("dropped session stops the spawned child within a finite deadline");
}
