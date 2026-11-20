// ---------------------------------------------------------------------------
// `/mode` typed over a running job
// ---------------------------------------------------------------------------

/// `/mode accept-edits` typed while the panel runs a job states the mode on the
/// job's cell through the same switch it states the session's: the job's next
/// call reads it, and the answer says so.
#[test]
fn slash_mode_moves_a_running_jobs_cell() {
    let mut fx = ChainFixture::build("mode-job");
    let cell = crate::agent::tools::shell_policy::ModeCell::new(ApprovalPolicy::Ask);

    assert!(matches!(
        dispatch_line_with_job("/mode accept-edits", &cell, &mut fx),
        Dispatch::Handled
    ));

    assert_eq!(
        fx.state.approval_mode, "accept-edits",
        "the session's own pair moved"
    );
    assert_eq!(
        cell.get(),
        ApprovalPolicy::AcceptEdits,
        "and the running job's cell moved with it"
    );
    let said = fx
        .app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        said.contains("The running job switched too"),
        "the mode line says the job got it: {said}"
    );
}

/// `/mode bypass` typed over a running job where this process's configuration
/// withholds the mode: the switch's own refusal is the whole answer, and
/// neither the session's pair nor the job's cell moves — the gate is one check
/// for both surfaces, not a rule the job's cell escapes.
#[test]
fn slash_mode_bypass_is_refused_over_a_running_job_without_the_key() {
    let mut fx = ChainFixture::build("mode-job-bypass");
    let cell = crate::agent::tools::shell_policy::ModeCell::new(ApprovalPolicy::Ask);
    assert!(
        !fx.state.allow_bypass,
        "the fixture's session withholds the mode"
    );

    assert!(matches!(
        dispatch_line_with_job("/mode bypass", &cell, &mut fx),
        Dispatch::Handled
    ));

    assert_eq!(fx.state.approval_mode, "ask", "the session stayed put");
    assert_eq!(
        cell.get(),
        ApprovalPolicy::Ask,
        "and the running job's cell stayed put with it"
    );
    let said = fx
        .app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        said.contains(crate::interactive::session::permissions::mode::BYPASS_REFUSAL),
        "the answer is the switch's own refusal: {said}"
    );
}
