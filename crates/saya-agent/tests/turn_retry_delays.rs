//! The loop's turn-level retry follows the backoff schedule its provider was
//! configured with (`[ai] retry_delays_ms`): the list's length is the number
//! of retries (at most eight) and its entries are the sleeps between them. The
//! server cuts every stream before `[DONE]`, so each attempt is retried until
//! the schedule runs out; the delays are read off the arrival times of the
//! requests.

#[path = "turn_retry_delays/support.rs"]
mod support;

use saya_agent::ProviderRecoveryPhase as Phase;
use std::time::Duration;
use support::{
    DEFAULT_DELAYS, Run, run_against_cut_streams, run_with_declared_schedule, unset_schedule,
};

fn ms(values: &[u64]) -> Vec<Duration> {
    values
        .iter()
        .map(|value| Duration::from_millis(*value))
        .collect()
}

/// Every retry waited at least its configured delay, and not the built-in one.
fn assert_slept(run: &Run, configured: &[Duration]) {
    let gaps = run.gaps();
    assert_eq!(gaps.len(), configured.len(), "attempt gaps: {gaps:?}");
    for (index, gap) in gaps.iter().enumerate() {
        assert!(
            *gap >= configured[index],
            "retry {} waited {gap:?}, less than its configured {:?}",
            index + 1,
            configured[index]
        );
        assert!(
            *gap < DEFAULT_DELAYS[index],
            "retry {} waited {gap:?}: the built-in schedule was slept instead of the configured one",
            index + 1
        );
    }
}

/// The reported progress counts the attempts that were really allowed.
fn assert_progress(run: &Run) {
    let attempts = u8::try_from(run.arrivals.len()).unwrap();
    let mut expected: Vec<_> = (2..=attempts)
        .map(|attempt| (Phase::Retrying, attempt, attempts))
        .collect();
    expected.push((Phase::Exhausted, attempts, attempts));
    assert_eq!(run.recoveries, expected);
}

#[tokio::test]
async fn an_eight_entry_schedule_makes_eight_retries_after_its_delays() {
    let configured = ms(&[20, 40, 60, 80, 100, 120, 140, 160]);
    let run = run_against_cut_streams(Some(configured.clone())).await;
    assert_eq!(run.arrivals.len(), 9, "one attempt plus eight retries");
    assert_slept(&run, &configured);
    assert_progress(&run);
}

#[tokio::test]
async fn a_longer_schedule_is_cut_to_eight_retries_using_its_first_eight_delays() {
    let run =
        run_with_declared_schedule(ms(&[20, 40, 60, 80, 100, 120, 140, 160, 3000, 3000])).await;
    assert_eq!(
        run.arrivals.len(),
        9,
        "the loop never retries more than eight times"
    );
    assert_slept(&run, &ms(&[20, 40, 60, 80, 100, 120, 140, 160]));
    assert_progress(&run);
}

#[tokio::test]
async fn a_one_entry_schedule_makes_one_retry_after_its_delay() {
    let run = run_against_cut_streams(Some(ms(&[30]))).await;
    assert_eq!(run.arrivals.len(), 2, "one attempt plus one retry");
    assert_slept(&run, &ms(&[30]));
    assert_progress(&run);
}

#[tokio::test]
async fn an_empty_schedule_makes_one_attempt_and_no_retry() {
    let run = run_against_cut_streams(Some(Vec::new())).await;
    assert_eq!(run.arrivals.len(), 1, "an empty schedule is `do not retry`");
    assert!(
        !run.recoveries
            .iter()
            .any(|(phase, ..)| *phase == Phase::Retrying),
        "nothing was retried: {:?}",
        run.recoveries
    );
    assert_progress(&run);
}

/// The default is asserted as a declaration, not by sleeping it: eight
/// entries total about ninety seconds.
#[test]
fn an_unset_schedule_declares_the_eight_entry_default_backoff() {
    assert_eq!(unset_schedule(), DEFAULT_DELAYS.to_vec());
}
