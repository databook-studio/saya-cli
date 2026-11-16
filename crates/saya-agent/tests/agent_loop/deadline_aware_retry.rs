/// With a wall clock shorter than the retry reserve, the first attempt has no
/// time to run and no retry may start. The job ends under the elapsed-time
/// budget class, not as a provider error.
#[tokio::test]
async fn a_retry_inside_the_reserve_ends_as_the_budget_stop() {
    let provider = RetryBudgetProvider::new(
        vec![Duration::ZERO],
        vec![RetryStep::Retryable, RetryStep::Retryable],
    );
    let started = std::time::Instant::now();
    let output = run_capped(&provider, Some(Duration::from_millis(100)))
        .await
        .expect("the reserve is a budget stop, not a provider failure");
    assert!(output.truncated, "the stop returns the partial work");
    assert!(
        stopped_on_the_clock(&output),
        "the stop names the elapsed-time budget: {:?}",
        output.events
    );
    assert!(
        provider.requests.lock().unwrap().len() <= 1,
        "no retry is opened inside the reserve"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the reserve, not the retry schedule, ends the run"
    );
}

/// The backoff never sleeps past the time the job has left minus the reserve.
/// A five-second schedule with one second of budget stops after one second.
#[tokio::test]
async fn the_backoff_is_clamped_to_the_remaining_budget() {
    let provider = RetryBudgetProvider::new(vec![Duration::from_secs(5)], vec![RetryStep::Retryable]);
    let started = std::time::Instant::now();
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        run_capped(&provider, Some(Duration::from_secs(31))),
    )
    .await
    .expect("the clamped backoff, not the five-second schedule, ends the run")
    .expect("a spent retry budget is a budget stop");
    assert!(output.truncated);
    assert!(
        stopped_on_the_clock(&output),
        "the stop names the elapsed-time budget: {:?}",
        output.events
    );
    assert_eq!(
        provider.requests.lock().unwrap().len(),
        1,
        "the reserve leaves no time for a second attempt"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the five-second backoff was clamped to the one-second budget"
    );
}

/// Each attempt's deadline is recomputed from what is left, not fixed once
/// when the logical request opens. A hanging second attempt is cut by the
/// shrunk deadline instead of the original 31.5 s.
#[tokio::test]
async fn each_attempt_deadline_shrinks_with_the_remaining_budget() {
    let provider = RetryBudgetProvider::new(
        vec![Duration::ZERO],
        vec![RetryStep::Retryable, RetryStep::Hang],
    );
    let started = std::time::Instant::now();
    let output = tokio::time::timeout(
        Duration::from_secs(4),
        run_capped(&provider, Some(Duration::from_millis(31_500))),
    )
    .await
    .expect("the second attempt must not wait for the original 31.5 s deadline")
    .expect("the wall clock is a budget stop");
    assert!(output.truncated);
    assert!(
        stopped_on_the_clock(&output),
        "the stop names the elapsed-time budget: {:?}",
        output.events
    );
    assert_eq!(
        provider.requests.lock().unwrap().len(),
        2,
        "the first failure retried, and the second attempt was cut"
    );
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the per-attempt deadline is the remaining budget minus the reserve"
    );
}

/// An uncapped job keeps the existing retry behaviour: no reserve, no
/// deadline shrink, and the scheduled retry succeeds.
#[tokio::test]
async fn an_uncapped_job_retries_unchanged() {
    let provider = RetryBudgetProvider::new(
        vec![Duration::ZERO],
        vec![RetryStep::Retryable, RetryStep::Answer],
    );
    let output = run_capped(&provider, None)
        .await
        .expect("an uncapped job recovers on its retry schedule");
    assert_eq!(output.answer, "answer");
    assert!(!output.truncated);
    assert!(!stopped_on_the_clock(&output));
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
}
