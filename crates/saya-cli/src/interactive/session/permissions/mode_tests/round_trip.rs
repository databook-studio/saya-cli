//! The stored posture round-trips: a mode the switch sets is the mode the
//! session's own readers read back, and the launch grammar still refuses the
//! session-only word.

use saya_agent::{
    ApprovalDecision, ApprovalPolicy, LocalStateEffect, PermissionMode, ToolEffect,
    WORKSPACE_WRITE_TOKEN,
};

use super::super::switch;
use super::support::Fixture;

/// The `write`/`edit` family's shape: a workspace file write, needing an
/// approval and carrying the token the mode's pre-answer names.
fn workspace_write() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::WriteWorkspace,
    }
}

/// `accept-edits` is the session's own word. `ApprovalPolicy`'s launch
/// grammar refuses it — that refusal is what keeps a flag, a config file and
/// a resumed record from reaching the posture — so the session's stored
/// spelling needs the session's own parse. Without it the switch is undone at
/// the next turn: the turn start re-reads the stored word, fails to parse it,
/// and syncs the session's one policy back to `ask`, while the bar keeps
/// naming a mode the engine is no longer in.
#[test]
fn a_switched_mode_survives_the_next_turns_read() {
    let mut fixture = Fixture::acquire("round-trip", ApprovalPolicy::Ask, false);
    switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::AcceptEdits,
    )
    .expect("accept-edits needs no gate");

    let read_back = fixture
        .state
        .approval_mode_parsed()
        .expect("the session's own word parses");
    assert_eq!(
        read_back,
        ApprovalPolicy::AcceptEdits,
        "the stored word round-trips through the session's own parse"
    );
    // The next turn's start, exactly: read the state, then sync the policy.
    fixture.session.sync_policy(read_back);
    assert_eq!(
        fixture.session.policy().mode(),
        ApprovalPolicy::AcceptEdits,
        "the sync leaves the posture where the switch put it"
    );
    assert_eq!(
        fixture
            .session
            .policy()
            .resolve(&workspace_write(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Allow,
        "and the workspace file family still runs unasked"
    );
}

/// The session's parse is not the launch grammar: the word the session can
/// store is still one no flag, config file or resumed record can state.
#[test]
fn the_session_word_is_still_not_a_launch_spelling() {
    assert!(
        "accept-edits".parse::<ApprovalPolicy>().is_err(),
        "the launch grammar must keep refusing the session-only spelling"
    );
}

/// Every other word keeps the launch grammar's refusal — the session's parse
/// widens the vocabulary by exactly one word, so a record carrying a spelling
/// no binary knows still fails the run rather than turning under a posture the
/// record never named. The readers that only display a mode fall back to the
/// closed direction instead; the readers that state one to the engine
/// propagate this refusal.
#[test]
fn an_unknown_stored_word_is_still_refused() {
    let mut fixture = Fixture::acquire("round-trip-unknown", ApprovalPolicy::Ask, false);
    fixture.state.approval_mode = "something-else".into();
    assert!(
        fixture.state.approval_mode_parsed().is_err(),
        "an unknown word is refused, never silently widened"
    );
    fixture.state.approval_mode = String::new();
    assert!(
        fixture.state.approval_mode_parsed().is_err(),
        "a legacy-absent word is refused too"
    );
    assert_eq!(
        super::super::current(&fixture.state),
        None,
        "and a reader that names a mode reads none rather than inventing one"
    );
}
