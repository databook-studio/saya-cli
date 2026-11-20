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
