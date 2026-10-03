use std::time::{Duration, Instant};

use saya_types::RunEvent;

use super::clock::{ElapsedClock, ElapsedClockError};

fn journal_clock(origin_unix_ms: u64, high_water_unix_ms: u64) -> Vec<RunEvent> {
    vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::WallClockObserved {
            origin_unix_ms,
            high_water_unix_ms,
        },
    ]
}

#[test]
fn resume_keeps_the_origin_and_raises_the_monotonic_high_water() {
    let instant = Instant::now();
    let mut clock = ElapsedClock::resume(
        &journal_clock(1_000, 1_200),
        Duration::from_secs(1),
        1_250,
        instant,
    )
    .unwrap();
    assert_eq!(clock.remaining(), Duration::from_millis(750));

    assert!(
        clock
            .observe(1_500, instant + Duration::from_millis(250))
            .unwrap()
    );
    assert_eq!(
        clock.event(),
        RunEvent::WallClockObserved {
            origin_unix_ms: 1_000,
            high_water_unix_ms: 1_500,
        }
    );
    assert!(
        clock
            .observe(1_550, instant + Duration::from_millis(500))
            .unwrap()
    );
    assert_eq!(
        clock.event(),
        RunEvent::WallClockObserved {
            origin_unix_ms: 1_000,
            high_water_unix_ms: 1_750,
        }
    );
    assert_eq!(clock.remaining(), Duration::from_millis(250));
}

#[test]
fn resume_rejects_missing_invalid_and_backwards_journal_time() {
    let now = Instant::now();
    let ceiling = Duration::from_secs(2);
    assert_eq!(
        ElapsedClock::resume(&[], ceiling, 2_000, now)
            .err()
            .unwrap(),
        ElapsedClockError::MissingOrigin
    );
    assert_eq!(
        ElapsedClock::resume(&journal_clock(2_000, 1_999), ceiling, 2_000, now,)
            .err()
            .unwrap(),
        ElapsedClockError::InvalidJournal
    );
    assert_eq!(
        ElapsedClock::resume(&journal_clock(2_000, 2_100), ceiling, 2_099, now,)
            .err()
            .unwrap(),
        ElapsedClockError::Backwards
    );
    assert_eq!(
        ElapsedClock::resume(&journal_clock(2_100, 2_100), ceiling, 2_099, now,)
            .err()
            .unwrap(),
        ElapsedClockError::FutureOrigin
    );
}

#[test]
fn exact_boundary_expires_and_a_forward_clock_jump_expires_conservatively() {
    let exact = ElapsedClock::resume(
        &journal_clock(1_000, 1_000),
        Duration::from_secs(1),
        2_000,
        Instant::now(),
    )
    .unwrap();
    assert!(exact.remaining().is_zero());

    let forward = ElapsedClock::resume(
        &journal_clock(1_000, 1_000),
        Duration::from_secs(1),
        5_000,
        Instant::now(),
    )
    .unwrap();
    assert!(forward.remaining().is_zero());
}

#[test]
fn observation_earlier_than_the_high_water_refuses_to_recharge() {
    let now = Instant::now();
    let mut clock = ElapsedClock::resume(
        &journal_clock(1_000, 1_200),
        Duration::from_secs(5),
        1_250,
        now,
    )
    .unwrap();
    assert_eq!(
        clock.observe(1_199, now + Duration::from_secs(1)),
        Err(ElapsedClockError::Backwards)
    );
    assert_eq!(clock.remaining(), Duration::from_millis(3_750));
}

#[test]
fn monotonic_submillisecond_elapsed_is_not_refunded_and_unrepresentable_high_water_fails_closed() {
    let now = Instant::now();
    let mut precise = ElapsedClock::arm(Duration::from_millis(2), 5_000, now).unwrap();
    precise
        .observe(5_000, now + Duration::from_micros(1_500))
        .unwrap();
    assert_eq!(precise.remaining(), Duration::from_micros(500));

    let origin = u64::MAX - 1;
    let mut overflowing = ElapsedClock::arm(Duration::from_millis(5), origin, now).unwrap();
    assert_eq!(
        overflowing.observe(origin, now + Duration::from_millis(5)),
        Err(ElapsedClockError::Unavailable)
    );
    assert!(overflowing.remaining().is_zero());
}
