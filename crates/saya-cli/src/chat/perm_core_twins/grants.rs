//! The grant a "yes for this run" records, on the live path: the run's own
//! store, fed by `ChatFront::ask` through `SessionOperator`, pre-answers the
//! call it names and nothing wider.
//!
//! Twins of `host_red_tests::grants::a_grant_pre_answers_any_argv_of_the_program_and_re_asks_any_other_program`
//! (the `command:<program>` flavour of that bound becomes the live `shell:`
//! prefix, the same bound spelled for a shell string) and of
//! `prompt_approval_tests::grant_lifecycle::{a_granted_token_stops_the_ask_without_a_prompt,
//! an_allow_once_answer_leaves_no_grant_behind}` (whose fixture is the old
//! session's `write`; the live write decider is `WriteAsk`).

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ApprovalPolicy, ToolDefinition, WORKSPACE_WRITE_TOKEN};
use serde_json::{Value, json};

use super::*;
use crate::agent::tools::shell_policy::ModeCell;
use crate::agent::tools::write_ask::WriteAsk;
use crate::approval_facts::ApprovalFacts;

/// A capability that admits every call it is asked about: the exact-write
/// admission is another unit's subject, so a refusal here can only be the
/// mode's or the store's.
struct Admits;

#[async_trait]
impl ApprovalDecider for Admits {
    async fn approve(&self, _tool: &ToolDefinition, _arguments: &Value) -> bool {
        true
    }
}

/// The facts a grantable `write` needs: a bound workspace root, the one member
/// the write family's token gates on.
fn rooted() -> ApprovalFacts {
    ApprovalFacts {
        workspace_root: Some(std::path::PathBuf::from("/home/user/proj")),
        ..ApprovalFacts::default()
    }
}

/// One `WriteAsk` over the run's operator, as the job root composes it.
fn write_ask(operator: &Arc<SessionOperator>) -> WriteAsk {
    let operator: Arc<dyn Operator> = operator.clone();
    WriteAsk::new(
        ModeCell::new(ApprovalPolicy::Ask),
        operator,
        &Caller::Session,
    )
}

/// A session-wide yes on the core shell records the question's own `shell:`
/// key, and the run then pre-answers every command under that prefix without a
/// second question — while a different program still asks.
#[tokio::test]
async fn a_session_yes_on_the_core_shell_pre_answers_the_same_prefix_and_re_asks_another() {
    let front = Scripted::answering(Answer::Session);
    let operator = operator(&front);
    let policy = live_shell(ApprovalPolicy::Ask, &operator, CommandRules::default());

    let first = json!({ "command": "npm install" });
    assert!(policy.approve(&first).await, "an admitted call runs");
    assert_eq!(front.asked(), 1, "the first call is put to the person");
    assert_eq!(
        front.questions()[0].grant.as_deref(),
        Some("shell:npm install"),
        "the question names the narrowest grant its call could record"
    );
    assert!(
        grants(&operator).is_granted("shell:npm install"),
        "a session-wide yes is remembered by the run: {:?}",
        grants(&operator).tokens()
    );

    for command in ["npm install --save-dev y", "npm install --legacy-peer-deps"] {
        assert!(
            policy.approve(&json!({ "command": command })).await,
            "any command under the granted prefix pre-answers: `{command}`"
        );
    }
    assert_eq!(
        front.asked(),
        1,
        "the held grant pre-answers every command under its prefix"
    );

    assert!(
        policy.approve(&json!({ "command": "make check" })).await,
        "a different program is admitted only once it has been asked"
    );
    assert_eq!(front.asked(), 2, "a different program still asks");
    assert_eq!(
        grants(&operator).tokens().len(),
        2,
        "the second yes records its own key, not the first one's"
    );
}

/// A granted `workspace-write` token stops a write ask: the same call the
/// person answered once is not put to them again, and there is no question at
/// all. Without the grant the same call is asked.
#[tokio::test]
async fn a_granted_write_is_not_asked_again_on_the_core_write_ask() {
    let tool = core_file_tool("write");
    let arguments = json!({ "path": "notes.md", "content": "hello" });
    let token = crate::grant_token::grant_token(&tool.name, &arguments, None, &rooted())
        .expect("the core write is grantable");
    assert_eq!(
        token, WORKSPACE_WRITE_TOKEN,
        "one token for both guarded file tools"
    );

    let front = Scripted::answering(Answer::Deny);
    let operator = operator(&front);
    let ask = write_ask(&operator);
    assert!(
        !ask.approve(&Admits, &tool, &arguments).await,
        "an ungranted ask whose person says no is refused"
    );
    assert_eq!(front.asked(), 1, "the ungranted call is put to the person");

    assert!(grants(&operator).grant(&token), "the first grant is new");
    assert!(
        ask.approve(&Admits, &tool, &arguments).await,
        "the granted token pre-answers the same shape"
    );
    assert_eq!(front.asked(), 1, "with no prompt at all");
}

/// "Allow once" leaves no grant behind: the store stays empty, so the same call
/// is put to the person again.
#[tokio::test]
async fn a_once_yes_on_the_core_write_ask_leaves_no_grant_behind() {
    let tool = core_file_tool("edit");
    let arguments = json!({
        "path": "notes.md",
        "old_text": "a",
        "new_text": "b",
        "expected_digest": "d",
    });
    let front = Scripted::answering(Answer::Once);
    let operator = operator(&front);
    let ask = write_ask(&operator);
    assert!(
        ask.approve(&Admits, &tool, &arguments).await,
        "allow once admits this call"
    );
    assert!(
        grants(&operator).is_empty(),
        "allow once records no grant: {:?}",
        grants(&operator).tokens()
    );
    assert!(ask.approve(&Admits, &tool, &arguments).await);
    assert_eq!(front.asked(), 2, "the same call asks again");
}

/// A call that names no offerable token puts no grant on the question, so no
/// session-wide answer is on offer for it — the ask never describes a grant the
/// person is not being offered.
#[tokio::test]
async fn a_core_call_with_no_offerable_grant_puts_no_grant_on_the_question() {
    let front = Scripted::answering(Answer::Session);
    let operator = operator(&front);
    let policy = live_shell(ApprovalPolicy::Ask, &operator, CommandRules::default());
    // A compound command is not one plain segment: there is no prefix to grant.
    let compound = json!({ "command": "cargo test && rm -rf x" });
    assert!(
        policy.approve(&compound).await,
        "the call is still put to a person"
    );
    assert_eq!(
        front.questions()[0].grant,
        None,
        "no key is guessed for a command that is not one plain segment"
    );
    assert!(
        grants(&operator).is_empty(),
        "a session-wide yes records nothing where no token was offered: {:?}",
        grants(&operator).tokens()
    );
}

/// The run's frozen decider cannot accumulate on the core write either: two
/// calls of the same ungranted shape both deny, and only the launch's own seed
/// pre-answers. `saya run` composes this decider, so the pin is the live one.
#[tokio::test]
async fn the_frozen_run_decider_cannot_accumulate_on_the_core_write() {
    use crate::prompt_approval::TerminalApproval;

    let tool = core_file_tool("write");
    let arguments = json!({ "path": "notes.md", "content": "hello" });
    let token = crate::grant_token::grant_token(&tool.name, &arguments, None, &rooted())
        .expect("the core write is grantable");
    let decider = TerminalApproval::frozen(ApprovalPolicy::Ask, &[], rooted());
    let first = decider.approve(&tool, &arguments).await;
    let second = decider.approve(&tool, &arguments).await;
    assert!(
        !first && !second,
        "an ungranted headless ask denies, twice: {first} {second}"
    );
    let with_seed = TerminalApproval::frozen(ApprovalPolicy::Ask, &[token], rooted());
    assert!(
        with_seed.approve(&tool, &arguments).await,
        "the seed pre-answers, and the seeds are the only grants a run holds"
    );
}
