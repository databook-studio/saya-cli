//! A shell command that names the repository's git directory is put to the
//! operator in the asking modes, even when a permission rule or a remembered
//! answer would otherwise pre-answer it. The vetted read-only set is unchanged
//! (a read cannot change the git directory), `bypass` is unchanged, and a deny
//! rule still decides first.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use saya_agent::{ApprovalPolicy, SessionGrants};
use saya_config::{CommandRule, CommandRules};
use serde_json::{Value, json};

use super::policy::{Lane, ShellPolicy};
use crate::agent::tools::consent::{Caller, Operator, Prompt};

/// An operator that counts the calls put to it and refuses every one.
struct Counting {
    calls: AtomicUsize,
}

impl Counting {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Operator for Counting {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        false
    }
}

fn prompt(operator: Arc<Counting>) -> Prompt {
    super::prompt(operator, Caller::Session)
}

fn rules(allow: &[&str], deny: &[&str]) -> CommandRules {
    CommandRules {
        allow: allow
            .iter()
            .map(|text| CommandRule::parse(text).expect("the test rule parses"))
            .collect(),
        deny: deny
            .iter()
            .map(|text| CommandRule::parse(text).expect("the test rule parses"))
            .collect(),
        diagnostics: Vec::new(),
        ..Default::default()
    }
}

fn policy(mode: ApprovalPolicy, operator: Arc<Counting>, rules: CommandRules) -> ShellPolicy {
    ShellPolicy::new(mode, Some(prompt(operator)), true, rules, None)
}

fn git_write() -> Value {
    json!({"command": "cp hook .git/hooks/pre-commit"})
}

/// An allow rule that covers the command does not pre-answer a write into the
/// git directory: `accept-edits` puts it to the operator.
#[tokio::test]
async fn an_allow_rule_does_not_pre_answer_a_git_write() {
    let operator = Counting::new();
    let policy = policy(
        ApprovalPolicy::AcceptEdits,
        operator.clone(),
        rules(&["cp"], &[]),
    );

    assert_eq!(policy.lane(&git_write()), Lane::Host(false));
    assert!(policy.would_ask(&git_write()));
    assert!(!policy.approve(&git_write()).await);
    assert_eq!(operator.calls(), 1, "the git write is put to the operator");
}

/// A remembered `shell:` answer is the same: it pre-answers the call the mode
/// would ask about, but not one that changes the git directory.
#[tokio::test]
async fn a_remembered_shell_answer_does_not_pre_answer_a_git_write() {
    let grants = SessionGrants::default();
    assert!(grants.grant("shell:cp hook"));
    let operator = Counting::new();
    let policy = ShellPolicy::new(
        ApprovalPolicy::AcceptEdits,
        Some(prompt(operator.clone())),
        true,
        CommandRules::default(),
        None,
    )
    .with_grants(Some(grants));

    assert_eq!(policy.lane(&git_write()), Lane::Host(false));
    assert!(!policy.approve(&git_write()).await);
    assert_eq!(operator.calls(), 1, "the remembered answer does not apply");
}

/// A redirect target is read the same as an argument: `echo hi > .git/config`
/// asks under `accept-edits`.
#[tokio::test]
async fn a_redirect_into_the_git_directory_asks() {
    let operator = Counting::new();
    let policy = policy(
        ApprovalPolicy::AcceptEdits,
        operator.clone(),
        CommandRules::default(),
    );
    let call = json!({"command": "echo hi > .git/config"});

    assert_eq!(policy.lane(&call), Lane::Host(false));
    assert!(!policy.approve(&call).await);
    assert_eq!(operator.calls(), 1);
}

/// The vetted read-only set is unchanged: a read of the git directory is not a
/// change to it, so it still skips the question.
#[tokio::test]
async fn a_vetted_read_of_the_git_directory_still_skips_the_question() {
    let operator = Counting::new();
    let policy = policy(
        ApprovalPolicy::AcceptEdits,
        operator.clone(),
        CommandRules::default(),
    );
    let call = json!({"command": "cat .git/config"});

    assert_eq!(policy.lane(&call), Lane::Host(true));
    assert!(!policy.would_ask(&call));
    assert!(policy.approve(&call).await);
    assert_eq!(operator.calls(), 0);
}

/// An ordinary command is unchanged: its allow rule still pre-answers it.
#[tokio::test]
async fn an_allow_rule_still_pre_answers_an_ordinary_command() {
    let operator = Counting::new();
    let policy = policy(
        ApprovalPolicy::AcceptEdits,
        operator.clone(),
        rules(&["cp"], &[]),
    );
    let call = json!({"command": "cp hook src/lib.rs"});

    assert_eq!(policy.lane(&call), Lane::Host(true));
    assert!(policy.approve(&call).await);
    assert_eq!(operator.calls(), 0);
}

/// `bypass` is unchanged: it admits the git write without asking.
#[tokio::test]
async fn bypass_admits_a_git_write_without_asking() {
    let operator = Counting::new();
    let policy = policy(
        ApprovalPolicy::Bypass,
        operator.clone(),
        rules(&["cp"], &[]),
    );

    assert!(policy.approve(&git_write()).await);
    assert_eq!(operator.calls(), 0, "bypass asks nobody");
}

/// A deny rule still decides first: a refused git write never reaches the
/// operator.
#[tokio::test]
async fn a_deny_rule_still_refuses_a_git_write() {
    let operator = Counting::new();
    let policy = policy(
        ApprovalPolicy::AcceptEdits,
        operator.clone(),
        rules(&[], &["cp"]),
    );

    assert!(matches!(policy.lane(&git_write()), Lane::Refused(_)));
    assert!(!policy.approve(&git_write()).await);
    assert_eq!(operator.calls(), 0);
}
