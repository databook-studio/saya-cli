/// `accept-edits` is `ask` plus one pre-answer of the mode's own: the
/// workspace file token `grant_token` suggests for `write` and `edit` runs
/// unasked, and every other call still reaches the frontend — including
/// every external side effect.
#[test]
fn accept_edits_pre_answers_the_workspace_file_token_only() {
    let policy = SessionPolicy::new(ApprovalPolicy::AcceptEdits);
    assert_eq!(
        policy.resolve(&write_shaped(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Allow,
        "the mode's own pre-answer: a workspace file write runs unasked"
    );
    for (effect, token, label) in [
        (
            write_shaped(),
            None,
            "a workspace write whose caller named no token",
        ),
        (
            side_effecting(),
            Some("fetch:example.com"),
            "http_fetch: an external side effect",
        ),
        (
            external_effect(),
            Some("command:cargo"),
            "run_command: an external side effect",
        ),
        (
            read_shaped(),
            Some("sql:analytics"),
            "a read-shaped call, which the mode still asks about",
        ),
    ] {
        assert_eq!(
            policy.resolve(&effect, token),
            ApprovalDecision::Ask,
            "accept-edits asks for everything but the file family: {label}"
        );
    }
}

/// The token is the key, not the shape: the mode answers exactly the token,
/// and re-derives nothing from the effect. That is what keeps `http_*` and
/// `run_command` asking — their callers name their own families' tokens —
/// and it rests on the caller's contract, the same one the grant store
/// already reads: `grant_token` names the family a call belongs to, so a
/// token is the call's family rather than a guess about its shape.
#[test]
fn accept_edits_keys_on_the_token_not_the_effect() {
    let policy = SessionPolicy::new(ApprovalPolicy::AcceptEdits);
    assert_eq!(
        policy.resolve(&side_effecting(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Allow,
        "a caller naming the file family is answered by the mode's pre-answer"
    );
    assert_eq!(
        policy.resolve(&write_shaped(), Some("fetch:example.com")),
        ApprovalDecision::Ask,
        "another family's token is not this mode's pre-answer"
    );
    assert_eq!(
        policy.resolve(&write_shaped(), None),
        ApprovalDecision::Ask,
        "and a call whose caller names no family asks"
    );
}

/// Nothing is recorded through the mode: no grant is written and no call is
/// counted under the token the pre-answer names, so a session listing shows
/// the user's own consents and nothing else.
#[test]
fn accept_edits_records_nothing() {
    let policy = SessionPolicy::new(ApprovalPolicy::AcceptEdits);
    assert_eq!(
        policy.resolve(&write_shaped(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Allow
    );
    assert_eq!(
        policy.resolve(&write_shaped(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Allow
    );
    assert!(
        !policy.grants().is_granted(WORKSPACE_WRITE_TOKEN),
        "the mode's pre-answer is the mode's, never a session grant"
    );
    assert_eq!(
        policy.grants().calls(WORKSPACE_WRITE_TOKEN),
        0,
        "the mode answered these calls, so the token's grant count is untouched"
    );
}

/// Plan narrows over accept-edits: the write the mode pre-answers denies under
/// the task posture. A `PermissionMode::Plan` switch leaves the consent
/// posture in place, so this is the pair `accept-edits → plan` composes.
#[test]
fn plan_denies_the_write_accept_edits_pre_answers() {
    let plan = with_mode(ApprovalPolicy::AcceptEdits, AgentMode::Plan);
    assert_eq!(
        plan.resolve(&write_shaped(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Deny { reason: None },
        "the task posture refuses the write before the mode's pre-answer runs"
    );
}
