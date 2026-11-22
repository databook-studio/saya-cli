//! What still reaches the `run_command` matcher in `deny.rs` — the counted
//! proof that its branch is not dead code.
//!
//! CAT kept `run_command` as a one-release alias of `shell`
//! (`saya_config::DEPRECATED_TOOL_ALIASES`) and kept `command:<program>` as the
//! legacy grant token a shell call still answers to. Both are user-facing for
//! 0.5.1, so the matcher that names the old door (`call_program`,
//! `denied_call_program`, `call_door`) stays until the alias does. These tests
//! state what reaches it, so a later removal cannot claim the branch was
//! unreachable:
//!
//! - the alias fold: a stated `run_command` is the `shell` (`canonical_tool_name`);
//! - the grant suggestion: a `run_command` call still names `command:<program>`;
//! - the deny door: the session executor refuses a `run_command` call naming a
//!   denied program before any lane, prompt, or grant is consulted.

use std::sync::Arc;

use serde_json::json;

use crate::interactive::session::permissions::deny::{
    SessionDeny, call_door, call_program, denied_call_program, denied_refusal,
};

/// The one-release alias: the old spelling is still accepted for a `[tools]
/// allow` statement, and the call it names still suggests the legacy
/// `command:<program>` grant the shell answers to.
#[test]
fn the_one_release_alias_still_maps_run_command_onto_the_shell() {
    assert_eq!(
        saya_config::canonical_tool_name("run_command"),
        "shell",
        "a stated `run_command` is the core's `shell`: the alias folds, never refuses"
    );
    assert_eq!(
        saya_config::tool_successor("run_command"),
        Some("shell"),
        "the alias table names the successor the fold uses"
    );
    assert!(
        saya_config::deprecation_note("run_command").is_some(),
        "the stale spelling is diagnosable (`config doctor`)"
    );
    // The grant a `run_command` call would record is still the program token
    // the shell reads on the other side of the alias.
    let facts = crate::approval_facts::ApprovalFacts {
        host: Some(crate::approval_facts::HostFacts::for_tests()),
        ..Default::default()
    };
    assert_eq!(
        crate::grant_token::grant_token(
            "run_command",
            &json!({"program": "npm", "args": ["test"]}),
            None,
            &facts,
        )
        .as_deref(),
        Some("command:npm"),
        "a `run_command` call still suggests the legacy program grant"
    );
}

/// The matcher answers one door and nothing else: the program is read from
/// `run_command`'s `program` argument, the journal's door is that same name,
/// and a `shell` call (a command line, not a program) gets no answer.
#[test]
fn the_program_matcher_answers_only_the_run_command_door() {
    assert_eq!(
        call_program("run_command", &json!({"program": "curl", "args": []})).as_deref(),
        Some("curl")
    );
    assert_eq!(
        call_program("shell", &json!({"command": "curl https://example"})),
        None,
        "the shell's door names a command line, never a program the deny list compares"
    );
    assert_eq!(call_program("run_command", &json!({"args": []})), None);
    assert_eq!(call_door("run_command"), "run_command");
    assert_eq!(call_door("shell"), "other");

    let denied = SessionDeny::from_names(["curl".to_owned()]).expect("a bare name binds");
    assert_eq!(
        denied_call_program(
            "run_command",
            &json!({"program": "curl"}),
            &denied.programs()
        )
        .as_deref(),
        Some("curl"),
        "the denied program is answered for the old door"
    );
    assert_eq!(
        denied_call_program(
            "run_command",
            &json!({"program": "wget"}),
            &denied.programs()
        ),
        None,
        "a program outside the list is not answered for"
    );
}

/// The branch is load-bearing at dispatch: the session executor refuses a
/// `run_command` call naming a denied program, with the pinned refusal, before
/// the host lane is reached. Without the matcher the call would fall through to
/// the lane and the deny list would refuse nothing at this door.
#[tokio::test]
async fn the_session_executor_refuses_a_denied_run_command_before_the_lane_runs() {
    use saya_agent::ToolExecutor;

    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let deny = SessionDeny::from_names(["curl".to_owned()]).expect("a bare name binds");
    let tools = crate::agent::tools::RunTools::compose(database, None).with_session_deny(deny);

    let error = tools
        .execute(
            "run_command",
            json!({"program": "curl", "args": ["-s", "https://example"]}),
        )
        .await
        .expect_err("a denied program must refuse at the door");
    assert!(
        error.to_string().contains(&denied_refusal("curl")),
        "the refusal is the deny list's own, not a lane failure: {error}"
    );

    let undeny = tools
        .execute("run_command", json!({"program": "wget", "args": []}))
        .await
        .expect_err("no host lane is composed: the call cannot run here");
    assert!(
        !undeny.to_string().contains("deny list"),
        "a program outside the list is not refused by deny: {undeny}"
    );
}
