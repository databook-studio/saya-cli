//! The name-only `command:` grant: what it pre-answers, what still asks,
//! and when it is offered at all.

use super::host_facts;
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::{grant_family, grant_token};
use serde_json::json;

/// One name-only grant: any argv under the granted program pre-answers, and
/// any other program still asks.
#[test]
fn a_grant_pre_answers_any_argv_of_the_program_and_re_asks_any_other_program() {
    let facts = host_facts();
    let policy = saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::Ask);
    assert!(
        policy.record(saya_agent::ApprovalChoice::AllowSession {
            token: "command:npm".to_owned(),
        }),
        "the first grant is new"
    );
    let tool = crate::interactive::session_definitions::run_command();
    for argv in [
        json!({"program": "npm", "args": ["install"]}),
        json!({"program": "npm", "args": ["run", "build"]}),
        json!({"program": "npm"}),
    ] {
        let token = grant_token(&tool.name, &argv, None, &facts);
        assert_eq!(
            token.as_deref(),
            Some("command:npm"),
            "any argv under the granted program suggests its token: {argv}"
        );
        assert_eq!(
            policy.resolve(&tool.effect, token.as_deref()),
            saya_agent::ApprovalDecision::Allow,
            "the held grant pre-answers any argv: {argv}"
        );
    }
    let other = json!({"program": "make", "args": ["check"]});
    assert_eq!(
        grant_token(&tool.name, &other, None, &facts),
        Some("command:make".to_owned()),
        "a different name suggests its own token"
    );
    let policy = saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::Ask);
    assert_eq!(
        policy.resolve(&tool.effect, Some("command:make")),
        saya_agent::ApprovalDecision::Ask,
        "a different program still asks"
    );
}

/// A shell- or interpreter-family name adds the warning with this lane's
/// clause. The token stays `command:`, never an interpreter-family word.
#[test]
fn interpreter_family_names_add_the_warning_and_command_tokens_never_interpreter_ones() {
    let facts = host_facts();
    let tool = crate::interactive::session_definitions::run_command();
    for program in ["bash", "python3", "node"] {
        let arguments = json!({"program": program, "args": ["-c", "echo hi"]});
        let token = grant_token(&tool.name, &arguments, None, &facts);
        assert_eq!(
            token,
            Some(format!("command:{program}")),
            "no family mirror: command:{program} is the token"
        );
        let body = crate::approval_facts::call_facts(
            "run_command",
            &arguments,
            Some(format!("command:{program}").as_str()),
            &facts,
            None,
            None,
        )
        .expect("the host lane states its own facts");
        assert!(
            body.contains("the model writes the program")
                && body.contains("there is no sandbox")
                && body.contains("it runs as you"),
            "the interpreter warning rides the prompt with this lane's clause: {body}"
        );
    }
}

/// A `command:` token is never offered where the lane did not compose.
#[test]
fn a_command_token_is_never_offered_where_the_lane_did_not_compose() {
    let tool = crate::interactive::session_definitions::run_command();
    let arguments = json!({"program": "npm", "args": ["install"]});
    assert_eq!(
        grant_token(&tool.name, &arguments, None, &ApprovalFacts::default()),
        None,
        "no composed lane, no command: offer"
    );
    let token = grant_token(&tool.name, &arguments, None, &host_facts());
    assert_eq!(
        token.as_deref(),
        Some("command:npm"),
        "the composed lane offers the name-only token"
    );
    assert_eq!(
        grant_family("command:npm"),
        Some("command"),
        "the session-history line groups the family"
    );
}
