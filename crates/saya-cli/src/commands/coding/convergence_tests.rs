use super::{DIRECT_STALL_LIMIT, Streak};

#[test]
fn a_run_of_failures_stalls_after_96_calls() {
    assert_eq!(DIRECT_STALL_LIMIT, 96);
}

#[test]
fn a_successful_call_is_a_baseline_not_a_failure() {
    assert_eq!(Streak::default().observe(false), 0);
}

#[test]
fn each_consecutive_failure_counts() {
    let mut streak = Streak::default();
    assert_eq!(streak.observe(true), 1);
    assert_eq!(streak.observe(true), 2);
    assert_eq!(streak.observe(true), 3);
}

#[test]
fn a_success_resets_the_run() {
    let mut streak = Streak::default();
    streak.observe(true);
    streak.observe(true);
    assert_eq!(streak.observe(false), 0);
    assert_eq!(streak.observe(true), 1);
}

#[test]
fn ninety_six_consecutive_failures_reach_the_bound() {
    let mut streak = Streak::default();
    let mut last = 0;
    for _ in 0..DIRECT_STALL_LIMIT {
        last = streak.observe(true);
    }
    assert_eq!(last, DIRECT_STALL_LIMIT);
}

/// However many calls a run makes, a success clears the run, so a root whose
/// calls all succeed never approaches the bound.
#[test]
fn successes_never_reach_the_bound() {
    let mut streak = Streak::default();
    let mut highest = 0;
    for index in 0..300 {
        highest = highest.max(streak.observe(index % 2 == 0));
    }
    assert_eq!(highest, 1, "only an isolated failure is ever counted");
}
