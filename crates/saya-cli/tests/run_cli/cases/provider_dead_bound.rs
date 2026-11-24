/// A run step against a provider that accepts and drops every connection is
/// bounded by the step's wall clock, with the DEFAULT retry schedule.
///
/// A dropped connection is a retryable transport failure, so the default
/// eight-delay schedule spends ~91.5 s per receive call; the engine's bounded
/// step retry re-runs that schedule for each of its three attempts, which is
/// ~287 s for one dead provider. The step's elapsed budget — inherited from
/// `[limits.run]` through `--budget wall-clock` — must reach the episode's
/// control, not only the engine's outer clock: the loop then refuses a retry
/// it cannot finish inside the budget, and the step ends within its bound
/// instead of walking the schedule.
#[test]
fn a_run_step_against_a_dead_provider_ends_within_its_bound() {
    let env = test_root("dead-provider-bound");
    // No `[ai] retry_delays_ms`: the product's own schedule is the point.
    let (dead, _ready) = mock(vec![Scripted {
        body: plan_body(&["a step the provider never answers"]),
        delay_ms: 0,
    }]);
    let started = std::time::Instant::now();
    let output = saya(
        &env,
        &[
            "--format",
            "ndjson",
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "--budget",
            "wall-clock=2",
            "a goal whose provider is dead",
        ],
        &dead,
    );
    let elapsed = started.elapsed();
    assert_eq!(
        output.status.code(),
        Some(6),
        "the step's bound pauses the run; stderr: {}",
        stderr(&output)
    );
    // The bound is the step's, so the whole invocation — process start, plan,
    // approval, and one step — fits well inside the schedule it must not walk.
    assert!(
        elapsed < Duration::from_secs(20),
        "one step against a dead provider ends within its 2 s bound, not inside \
         3 × 91.5 s of retry schedule: took {elapsed:?}"
    );
    let out = json_lines(&stdout(&output), "stdout");
    let paused = out
        .iter()
        .find(|value| value.get("type").and_then(|tag| tag.as_str()) == Some("paused"))
        .expect("a paused line");
    assert_eq!(
        paused.get("reason").and_then(|reason| reason.as_str()),
        Some("wall_clock_exceeded"),
        "the pause names the elapsed budget it spent: {paused:?}"
    );
    // A retry the bound cannot finish is never started: the stream carries the
    // recovery note that refused it, and no retry the schedule would sleep on.
    let retrying = out
        .iter()
        .filter(|value| {
            value.get("event").and_then(|tag| tag.as_str()) == Some("provider_recovery")
                && value.get("phase").and_then(|phase| phase.as_str()) == Some("retrying")
        })
        .count();
    assert_eq!(
        retrying, 0,
        "the step starts no provider retry its bound cannot finish: {out:?}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
