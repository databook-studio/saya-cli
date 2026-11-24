//! `--deny <program>` binding the `shell` tool (SEC1): the launch converts
//! each denied program into a deny rule in the trusted rule set the shell
//! policy decides first in every mode, and the one matcher catches the
//! spellings a program arrives in — without blocking a name that merely
//! resembles it.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::ApprovalPolicy;
use saya_config::CommandRules;
use serde_json::{Value, json};

use super::deny::{self, SessionDeny};
use crate::agent::tools::consent::{Caller, Operator, Prompt};
use crate::agent::tools::shell_policy::ShellPolicy;

/// An operator that approves everything: only the deny rule can refuse a call
/// it is present for, so its presence proves the rule decides first.
struct AlwaysApprove;

#[async_trait]
impl Operator for AlwaysApprove {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        true
    }
}

pub(super) fn approving_prompt() -> Prompt {
    crate::agent::tools::shell_policy::prompt(Arc::new(AlwaysApprove), Caller::Session)
}

/// The rules a launch stating `--deny <programs>` hands the session's shell.
pub(super) fn launched(programs: &[&str]) -> CommandRules {
    let deny = SessionDeny::from_names(programs.iter().map(|program| (*program).to_owned()))
        .expect("a bare name builds");
    deny::shell_rules(&CommandRules::default(), &deny).expect("the deny list converts")
}

/// The session's shell policy over those rules, with a sandbox lane present so
/// the asking modes have somewhere to reroute a call they cannot refuse.
pub(super) fn shell(
    mode: ApprovalPolicy,
    prompt: Option<Prompt>,
    rules: &CommandRules,
) -> ShellPolicy {
    ShellPolicy::new(mode, prompt, true, rules.clone(), None)
}

/// One form must be refused by the rule and must not be approved: every
/// assertion names its own command, so a gap is attributable to one spelling.
async fn refused(policy: &ShellPolicy, command: &str) {
    let call = json!({ "command": command });
    let reason = policy
        .refusal_detail(&call)
        .unwrap_or_else(|| panic!("`{command}` must be refused by the deny rule"));
    assert!(
        reason.contains("denied by the permission rule `curl`"),
        "`{command}` must carry the shell lane's existing rule wording: {reason}"
    );
    assert!(
        !policy.approve(&call).await,
        "`{command}` must not be approved"
    );
}

/// 1. `--deny curl` refuses `shell {command: "curl x"}` under `bypass`, `ask`
///    and `accept-edits` — even where an operator would have approved it, and
///    even headless, where `ask` would otherwise reroute the call to the
///    sandbox unasked.
#[tokio::test]
async fn a_launch_deny_refuses_the_shell_tool_in_bypass_ask_and_accept_edits() {
    let rules = launched(&["curl"]);
    for mode in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
    ] {
        for prompt in [None, Some(approving_prompt())] {
            let policy = shell(mode, prompt, &rules);
            let call = json!({ "command": "curl x" });
            assert!(
                policy.refusal_detail(&call).is_some(),
                "{mode:?} must refuse a denied program"
            );
            assert!(
                !policy.approve(&call).await,
                "{mode:?} must not run a denied program unasked"
            );
            assert!(
                !policy.would_ask(&call),
                "{mode:?} refuses: it does not put a denied program to the operator"
            );
        }
    }
}

/// 2. The spellings a denied program arrives in, each its own assertion. Every
///    one is decided by the one matcher; where the matcher cannot read a form
///    the shell must still not run it unasked under `bypass`.
#[tokio::test]
async fn the_one_matcher_catches_the_spellings_a_denied_program_arrives_in() {
    let rules = launched(&["curl"]);
    // `bypass` is the mode with no question left to ask: a form it runs is a
    // form the deny list does not bound.
    let policy = shell(ApprovalPolicy::Bypass, None, &rules);
    refused(&policy, "curl").await;
    refused(&policy, "/usr/bin/curl x").await;
    refused(&policy, "echo hi && curl x").await;
    refused(&policy, "echo hi; curl x").await;
    refused(&policy, "env curl x").await;
    refused(&policy, "FOO=1 curl x").await;
    refused(&policy, "(curl x)").await;
    refused(&policy, "$(curl x)").await;
    refused(&policy, "`curl x`").await;
    // The spaced spellings of the same two openers name the same program.
    refused(&policy, "$( curl x )").await;
    refused(&policy, "` curl x `").await;
}

/// 3. No over-blocking: a program the list does not name still runs under
///    `bypass`, including a name that merely begins with a denied one and a
///    program named after it as an argument.
#[tokio::test]
async fn a_program_the_list_does_not_name_still_runs_under_bypass() {
    let rules = launched(&["curl"]);
    let policy = shell(ApprovalPolicy::Bypass, None, &rules);
    for command in ["curly", "curly x", "echo curl"] {
        let call = json!({ "command": command });
        assert!(
            policy.refusal_detail(&call).is_none(),
            "`{command}` is not on the list and must not be refused"
        );
        assert!(
            policy.approve(&call).await,
            "`{command}` still runs under bypass"
        );
    }
}

/// 4. The launch's statement is still journaled as the deny list, and
///    `run_command`'s deny path is unchanged by the conversion: the rule set
///    gains the program, the door check still reads the list itself.
#[test]
fn the_deny_list_still_journals_and_still_refuses_run_command() {
    let deny = SessionDeny::from_names(["curl".to_owned()]).expect("a bare name builds");
    let rules = deny::shell_rules(&CommandRules::default(), &deny).expect("the list converts");
    assert_eq!(
        rules
            .deny
            .iter()
            .map(saya_config::CommandRule::text)
            .collect::<Vec<_>>(),
        vec!["curl"],
        "the program is one deny rule in the trusted set"
    );
    let run = json!({ "program": "curl", "args": ["-s"] });
    assert_eq!(
        deny::denied_call_program("run_command", &run, &deny.programs()),
        Some("curl".to_owned()),
        "run_command's own deny path reads the list as before"
    );
    let dir = std::path::Path::new("target").join(format!("tmp-deny-shell-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the state directory creates");
    let journal = saya_store::SessionJournal::open(&dir);
    journal
        .deny_list(&deny.programs())
        .expect("the start event journals");
    assert_eq!(
        journal.read().expect("the journal reads"),
        vec![saya_store::JournalEvent::DenyList {
            programs: vec!["curl".to_owned()],
        }],
        "the launch still journals `session-deny-list` once, unchanged"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
