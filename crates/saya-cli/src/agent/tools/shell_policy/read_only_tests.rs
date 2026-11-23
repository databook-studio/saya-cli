//! Where the vetted read-only set meets the lane: an asking mode skips the
//! question for a command that only reads, the deny list still wins, and the
//! switch turns the whole thing off.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use saya_agent::ApprovalPolicy;
use saya_config::{CommandRule, CommandRules, ReadOnlySet};
use serde_json::{Value, json};

use super::policy::{HOST_APPROVAL_UNAVAILABLE, Lane, SANDBOX_DOWNGRADE_NOTICE, ShellPolicy};
use crate::agent::tools::consent::{Caller, Operator};

/// An operator that refuses every call and counts how often it was asked, so
/// "no question" is a fact about the person, not about the verdict.
struct Counting {
    asked: Arc<AtomicUsize>,
}

impl Counting {
    fn new() -> (Arc<Self>, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                asked: Arc::clone(&asked),
            }),
            asked,
        )
    }
}

#[async_trait]
impl Operator for Counting {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        self.asked.fetch_add(1, Ordering::Relaxed);
        false
    }
}

fn reviewing(policy: ApprovalPolicy, rules: CommandRules) -> (ShellPolicy, Arc<AtomicUsize>) {
    let (operator, asked) = Counting::new();
    let prompt = super::prompt(operator, Caller::Session);
    let shell = ShellPolicy::new(policy, Some(prompt), true, rules, None);
    (shell, asked)
}

fn call(command: &str) -> Value {
    json!({ "command": command })
}

fn deny(programs: &[&str]) -> CommandRules {
    CommandRules {
        deny: programs
            .iter()
            .map(|text| CommandRule::parse(text).expect("the test rule parses"))
            .collect(),
        ..Default::default()
    }
}

/// The finding this change answers: under `ask`, the model's own exploration
/// runs with no question at all — the operator is there and is not reached.
#[tokio::test]
async fn a_vetted_read_only_command_runs_on_the_host_without_a_question() {
    for command in [
        "ls -la",
        "cat README.md",
        "grep -n foo src/main.rs",
        "ls -la | wc -l",
    ] {
        let (shell, asked) = reviewing(ApprovalPolicy::Ask, CommandRules::default());
        let call = call(command);
        assert_eq!(
            shell.lane(&call),
            Lane::Host(true),
            "`{command}` is the set's own answer"
        );
        assert!(
            !shell.would_ask(&call),
            "`{command}` puts no question to a person"
        );
        assert!(
            shell.approve(&call).await,
            "`{command}` runs: a refusing operator never saw it"
        );
        assert_eq!(asked.load(Ordering::Relaxed), 0, "`{command}` asked");
    }
}

/// `accept-edits` asks exactly like `ask` about a command's effects, so the set
/// answers there too — and nowhere else.
#[tokio::test]
async fn accept_edits_skips_the_same_questions() {
    let (shell, asked) = reviewing(ApprovalPolicy::AcceptEdits, CommandRules::default());
    let call = call("ls -la");
    assert_eq!(shell.lane(&call), Lane::Host(true));
    assert!(shell.approve(&call).await);
    assert_eq!(asked.load(Ordering::Relaxed), 0);
}

/// Everything outside the set asks as it always did.
#[tokio::test]
async fn a_command_outside_the_set_still_asks() {
    for command in [
        "npm install",
        "ls > out.txt",
        "git push",
        "rm -rf build",
        // The finding this pass answers: a vetted spelling of a program is not
        // a licence to read what the workspace tools refuse, to expand, or to
        // read a tree of hidden files.
        "cat .env",
        "cat ~/.ssh/id_rsa",
        "head ../other-repo/.env",
        "echo $OPENAI_API_KEY",
        "grep -r KEY .",
    ] {
        let (shell, asked) = reviewing(ApprovalPolicy::Ask, CommandRules::default());
        let call = call(command);
        assert_eq!(
            shell.lane(&call),
            Lane::Host(false),
            "`{command}` is not in the set"
        );
        assert!(shell.would_ask(&call), "`{command}` asks");
        assert!(
            !shell.approve(&call).await,
            "`{command}` reaches the operator, whose answer stands"
        );
        assert_eq!(asked.load(Ordering::Relaxed), 1, "`{command}` asked once");
    }
}

/// The read tool's private-path set is one rule, not the configuration's alone:
/// the vetted set must not read what the workspace tools refuse. A path the
/// store's screen or the shared redactor refuses — and `sensitive_path` by
/// itself allows — is still a path the shell asks about.
#[tokio::test]
async fn a_private_path_outside_the_sensitive_set_still_asks() {
    let private = [
        // The store's credential directories: `.docker` is deliberately not in
        // the configuration's own set (it is ordinary repository content).
        ".docker/config.json",
        // A database export the screen refuses by extension.
        "data/app.sqlite3",
    ];
    for path in private {
        assert!(
            saya_harness::workspace::private_path(path),
            "the workspace's own rule refuses `{path}`"
        );
        assert!(
            !saya_config::sensitive_path(path),
            "and the configuration's set alone allows it, which is the gap"
        );
    }
    for command in [
        "cat .docker/config.json",
        "head data/app.sqlite3",
        "tail -n 5 data/app.sqlite3",
    ] {
        let (shell, asked) = reviewing(ApprovalPolicy::Ask, CommandRules::default());
        let call = call(command);
        assert_eq!(
            shell.lane(&call),
            Lane::Host(false),
            "`{command}` names a path the read tool refuses"
        );
        assert!(shell.would_ask(&call), "`{command}` asks");
        assert!(
            !shell.approve(&call).await,
            "`{command}` reaches the operator, whose answer stands"
        );
        assert_eq!(asked.load(Ordering::Relaxed), 1, "`{command}` asked once");
    }
}

/// The deny list always wins: a denied program in the set is refused, and the
/// operator is never asked about it.
#[tokio::test]
async fn a_denied_program_in_the_set_is_refused() {
    let (shell, asked) = reviewing(ApprovalPolicy::Ask, deny(&["ls"]));
    let call = call("ls -la");
    let lane = shell.lane(&call);
    assert!(
        matches!(&lane, Lane::Refused(reason) if reason.contains("`ls`")),
        "a denied program is refused whatever the set says: {lane:?}"
    );
    assert!(!shell.would_ask(&call), "a refusal is not a question");
    assert!(!shell.approve(&call).await);
    assert_eq!(asked.load(Ordering::Relaxed), 0);
    // The same command without the denial is the set's own answer.
    let (vetted, _) = reviewing(ApprovalPolicy::Ask, CommandRules::default());
    assert_eq!(vetted.lane(&call), Lane::Host(true));
}

/// The switch is the cautious user's answer: off means every command asks
/// again, the vetted ones included.
#[tokio::test]
async fn the_switch_off_asks_for_everything() {
    let rules = CommandRules {
        read_only_set: ReadOnlySet::Off,
        ..Default::default()
    };
    let (shell, asked) = reviewing(ApprovalPolicy::Ask, rules);
    let call = call("ls -la");
    assert_eq!(shell.lane(&call), Lane::Host(false));
    assert!(shell.would_ask(&call));
    assert!(!shell.approve(&call).await);
    assert_eq!(asked.load(Ordering::Relaxed), 1);
}

/// The set skips a question, not the consent path: with no operator to skip,
/// the lane falls back exactly as before — the sandbox when one is proven, and
/// today's refusal when none is.
#[test]
fn with_no_operator_the_fallback_stands() {
    let sandboxed = ShellPolicy::new(
        ApprovalPolicy::Ask,
        None,
        true,
        CommandRules::default(),
        None,
    );
    assert_eq!(
        sandboxed.lane(&call("ls -la")),
        Lane::Sandbox(Some(SANDBOX_DOWNGRADE_NOTICE)),
        "nobody could be asked, so the proven sandbox takes the call"
    );
    let hostless = ShellPolicy::new(
        ApprovalPolicy::Ask,
        None,
        false,
        CommandRules::default(),
        None,
    );
    assert_eq!(
        hostless.lane(&call("ls -la")),
        Lane::Refused(HOST_APPROVAL_UNAVAILABLE.to_owned()),
        "and with no sandbox either, today's refusal stands"
    );
}

/// The modes that already decide are untouched: `read-only` keeps the sandbox,
/// `never` runs no shell, and `bypass` never asks whatever the set says.
#[test]
fn the_modes_that_already_decide_are_unchanged() {
    let (operator, _) = Counting::new();
    let read_only = ShellPolicy::new(
        ApprovalPolicy::ReadOnly,
        Some(super::prompt(operator.clone(), Caller::Session)),
        true,
        CommandRules::default(),
        None,
    );
    assert_eq!(read_only.lane(&call("ls -la")), Lane::Sandbox(None));
    let never = ShellPolicy::new(
        ApprovalPolicy::Never,
        Some(super::prompt(operator.clone(), Caller::Session)),
        true,
        CommandRules::default(),
        None,
    );
    assert!(matches!(never.lane(&call("ls -la")), Lane::Refused(_)));
    let bypass = ShellPolicy::new(
        ApprovalPolicy::Bypass,
        Some(super::prompt(operator, Caller::Session)),
        true,
        CommandRules::default(),
        None,
    );
    assert_eq!(bypass.lane(&call("npm install")), Lane::Host(false));
}

/// A sandboxed call is answered by its confinement, not by the set: the same
/// command takes the same lane whether or not it is vetted.
#[test]
fn a_sandboxed_call_is_decided_by_its_own_confinement() {
    let (shell, asked) = reviewing(ApprovalPolicy::Ask, CommandRules::default());
    let sandboxed = json!({ "command": "npm install", "sandbox": true });
    assert_eq!(shell.lane(&sandboxed), Lane::Sandbox(None));
    assert!(!shell.would_ask(&sandboxed));
    assert_eq!(asked.load(Ordering::Relaxed), 0);
}
