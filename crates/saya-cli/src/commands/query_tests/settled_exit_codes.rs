/// A Budget(ElapsedTime)-latched question that still produced a natural
/// output (the deadline expired at/after verification) exits 6 — the
/// partial/stopped class — and the printed diagnostic says the answer is
/// partial. Before the repair this returned 0 with no wording.
#[test]
fn a_budget_latched_question_with_a_natural_answer_exits_six_with_partial_wording() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    capture_output_start();
    let code = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code, not an error");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(
        code, 6,
        "the budget-stopped class is the paused exit: {code}"
    );
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("partial"),
        "the diagnostic says the answer is partial/stopped: {printed:?}"
    );
}

/// A User-latched token on the work-ready branch exits 130 — the same class
/// the ready-signal branch returns — so a ready-work/ready-signal race
/// classifies the same either way. Before the repair this returned 0.
#[test]
fn a_user_latched_question_exits_130_on_the_work_ready_branch() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let code = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code");
    assert_eq!(
        code, 130,
        "a user-stopped run is the interrupted class: {code}"
    );
}

/// A budget stop before any answer existed (setup error) still classifies
/// from the token: exit 6, without inventing output — and the wording stays
/// truthful that nothing finished.
#[test]
fn a_budget_latched_setup_failure_exits_six_without_inventing_an_answer() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    capture_output_start();
    let code = settle_ask(
        Err(AgentRuntimeError::Agent("request cancelled".into())),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(
        code, 6,
        "the budget-stopped class on the error branch: {code}"
    );
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("partial"),
        "the diagnostic stays truthful about the unfinished answer: {printed:?}"
    );
}

/// A user stop before any answer existed exits 130 too.
#[test]
fn a_user_latched_setup_failure_exits_130() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let code = settle_ask(
        Err(AgentRuntimeError::Agent("request cancelled".into())),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    assert_eq!(
        code, 130,
        "a user-stopped setup is the interrupted class: {code}"
    );
}

/// An OwnerFailure stop on the work-ready branch exits 5 with a message that
/// says what actually stopped the run.
#[test]
fn an_owner_failure_on_the_work_ready_branch_exits_five_with_a_truthful_message() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
    capture_output_start();
    let code = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(code, 5, "an owner failure is the failure class: {code}");
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("owner failure"),
        "the diagnostic names the owner failure: {printed:?}"
    );
}

/// An OwnerFailure stop on the setup-error branch keeps exit 5 and keeps the
/// run's own error as the message — the truth the owner failure interrupted.
#[test]
fn an_owner_failure_on_a_setup_error_keeps_exit_five_with_the_run_s_own_error() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
    capture_output_start();
    let code = settle_ask(
        Err(AgentRuntimeError::Provider("provider down".into())),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(code, 5);
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("provider down"),
        "the run's own error is the message: {printed:?}"
    );
}

/// A question that stopped for no typed reason keeps the existing exits:
/// a natural answer 0, a settled clarification 6.
#[test]
fn a_question_that_stopped_for_no_reason_keeps_the_existing_exit() {
    let cancellation = CancellationToken::new();
    let natural = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code");
    assert_eq!(natural, 0, "an ordinary completion still exits 0");
    let paused = settle_ask(
        Ok(clarification_output()),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    assert_eq!(paused, 6, "a settled clarification still exits 6");
}

/// A Ctrl-C that arrives AFTER a budget stop already latched the token must
/// keep the budget class (6, partial wording), not report a user interrupt.
#[test]
fn a_ctrl_c_after_a_budget_latch_keeps_the_budget_class() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    cancellation.cancel();
    capture_output_start();
    let code = super::interrupted_exit(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("an interrupted ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(code, 6, "the first stop reason wins: {code}");
    assert!(format!("{stdout}{stderr}").contains("partial"));
}

/// A plain Ctrl-C (the token carries the user's stop) is still exit 130 and
/// prints nothing extra, exactly as before.
#[test]
fn a_plain_ctrl_c_still_exits_130_quietly() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    capture_output_start();
    let code = super::interrupted_exit(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("an interrupted ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(code, 130);
    assert!(
        format!("{stdout}{stderr}").is_empty(),
        "no new output on a plain interrupt"
    );
}
