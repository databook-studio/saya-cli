/// Provider error → unchanged conversation, failure message, session still
/// usable: the error is said plainly and nothing is stored.
#[tokio::test]
async fn a_provider_error_leaves_the_session_usable() {
    let state = turns(5);
    let before = state.provider_history();
    let plan = plan(&state.turns, &before).expect("long history plans");
    let error = summarise(&ErrorProvider, "m", &plan)
        .await
        .expect_err("a provider error must fail compaction");
    assert!(
        error.contains("summariser errored"),
        "the reason is plain: {error}"
    );
    let message = failure_message(&error);
    assert!(
        message.contains("Compaction failed") && message.contains("unchanged"),
        "the failure message says what happened and the guarantee: {message}"
    );
    assert_eq!(
        state.provider_history(),
        before,
        "the conversation is unchanged after a provider error"
    );
}

/// Truncated summary → same: unchanged conversation, failure message, and the
/// reason names the truncation rather than a generic error.
#[tokio::test]
async fn a_truncated_summary_leaves_the_session_unchanged() {
    let state = turns(5);
    let before = state.provider_history();
    let plan = plan(&state.turns, &before).expect("long history plans");
    let error = summarise(&TruncatedProvider, "m", &plan)
        .await
        .expect_err("a truncated summary must fail compaction");
    assert!(
        error.contains("truncated"),
        "the reason names the truncation: {error}"
    );
    assert_eq!(
        state.provider_history(),
        before,
        "the conversation is unchanged after a truncation"
    );
}
