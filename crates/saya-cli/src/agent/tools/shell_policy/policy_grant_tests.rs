//! A job's remembered "yes for this run": the `shell:` grant an `AllowSession`
//! recorded pre-answers the calls it names — and only the calls it names, only
//! under the modes that would have asked, and never ahead of a deny rule. That
//! order is what these pin: a store consulted anywhere but after the rules and
//! the mode turns a remembered answer into a bypass.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use saya_agent::{ApprovalPolicy, SessionGrants};
use saya_config::{CommandRule, CommandRules};
use saya_types::{AgentId, JobId};
use serde_json::{Value, json};

use super::ModeCell;
use super::policy::{Lane, NEVER_REFUSAL, READ_ONLY_REFUSAL, ShellPolicy};
use crate::agent::tools::consent::{Caller, Operator, Prompt};

/// One job's operator: the store it remembers answers in, and how many calls
/// were put to it. It refuses everything it is asked, so an admitted call can
/// only be one the store — or the mode — pre-answered.
struct Remembering {
    grants: SessionGrants,
    asked: AtomicUsize,
}

impl Remembering {
    fn holding(tokens: &[&str]) -> Arc<Self> {
        let grants = SessionGrants::default();
        for token in tokens {
            grants.grant(token);
        }
        Arc::new(Self {
            grants,
            asked: AtomicUsize::new(0),
        })
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Operator for Remembering {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        false
    }

    fn grants(&self) -> Option<SessionGrants> {
        Some(self.grants.clone())
    }
}

/// No rules: every call is decided as if the table were absent.
fn no_rules() -> CommandRules {
    CommandRules::default()
}

fn rules(deny: &[&str]) -> CommandRules {
    CommandRules {
        allow: Vec::new(),
        deny: deny
            .iter()
            .map(|text| CommandRule::parse(text).expect("the test rule parses"))
            .collect(),
        diagnostics: Vec::new(),
    }
}

/// A caller's prompt over one job's operator: the store reaches the policy
/// only through the prompt its operator built.
fn prompt(operator: &Arc<Remembering>, caller: Caller) -> Prompt {
    let operator: Arc<dyn Operator> = operator.clone();
    crate::agent::tools::shell_policy::prompt(operator, caller)
}

/// One shell of a job: its mode, its operator's store and whether a sandbox
/// was proven.
fn job_shell(
    policy: ApprovalPolicy,
    operator: &Arc<Remembering>,
    has_sandbox: bool,
    rules: CommandRules,
) -> ShellPolicy {
    ShellPolicy::new(
        policy,
        Some(prompt(operator, Caller::Session)),
        has_sandbox,
        rules,
        None,
    )
}

/// A helper's shell of the same job, under the helper's own subject line.
fn helper_shell(operator: &Arc<Remembering>) -> ShellPolicy {
    let caller = Caller::Helper {
        agent_id: AgentId::parse("helper-2").expect("the test helper id parses"),
        job_id: JobId::parse("job-a").expect("the test job id parses"),
    };
    ShellPolicy::new(
        ApprovalPolicy::Ask,
        Some(prompt(operator, caller)),
        true,
        no_rules(),
        None,
    )
}

/// The next call sharing the remembered `shell:` prefix is admitted with no
/// question, and a call outside it still asks.
#[tokio::test]
async fn a_remembered_grant_answers_the_next_call_without_asking() {
    let operator = Remembering::holding(&["shell:cargo test"]);
    let shell = job_shell(ApprovalPolicy::Ask, &operator, true, no_rules());
    let next = json!({ "command": "cargo test -p y" });
    assert_eq!(
        shell.lane(&next),
        Lane::Host(true),
        "the stored grant pre-answers the call"
    );
    assert!(shell.approve(&next).await, "a remembered grant admits it");
    assert_eq!(operator.asked(), 0, "and puts no question to a person");

    let other = json!({ "command": "cargo build" });
    assert_eq!(
        shell.lane(&other),
        Lane::Host(false),
        "a call outside the prefix still asks"
    );
    assert!(
        !shell.approve(&other).await,
        "the operator's refusal is what it gets"
    );
    assert_eq!(operator.asked(), 1, "cargo build still asks");
}

/// A command that is not one plain segment names no token of its own, so no
/// stored token — not even the broader `shell:cargo` — admits it.
#[tokio::test]
async fn a_remembered_grant_never_covers_a_compound_call() {
    let operator = Remembering::holding(&["shell:cargo test", "shell:cargo"]);
    let shell = job_shell(ApprovalPolicy::Ask, &operator, true, no_rules());
    let commands = [
        "cargo test && rm -rf x",
        "cargo test | tee o",
        "cargo test > o",
    ];
    for command in commands {
        let call = json!({ "command": command });
        assert_eq!(
            shell.lane(&call),
            Lane::Host(false),
            "`{command}` names no token to match"
        );
        assert!(
            !shell.approve(&call).await,
            "`{command}` must not be admitted by a prefix"
        );
    }
    assert_eq!(operator.asked(), commands.len(), "each of them asks");
}

/// A deny rule refuses first, in every mode: a remembered token for the same
/// prefix cannot revive the call, and the operator is never reached.
#[tokio::test]
async fn a_deny_rule_outranks_a_remembered_grant() {
    let operator = Remembering::holding(&["shell:git stash"]);
    for mode in [ApprovalPolicy::Ask, ApprovalPolicy::Bypass] {
        let shell = job_shell(mode, &operator, true, rules(&["git stash"]));
        for command in ["git stash list", "git stash push"] {
            let call = json!({ "command": command });
            let lane = shell.lane(&call);
            assert!(
                matches!(&lane, Lane::Refused(reason) if reason.contains("`git stash`")),
                "{mode:?} must refuse `{command}` and name the rule: {lane:?}"
            );
            assert!(
                !shell.approve(&call).await,
                "{mode:?} must not admit `{command}`"
            );
        }
    }
    assert_eq!(operator.asked(), 0, "a denied call never reaches a person");
}

/// The mode decides after the rules and before anything remembered: `read-only`
/// keeps the call off the host, `never` runs no shell, and neither asks.
#[tokio::test]
async fn the_mode_outranks_a_remembered_grant() {
    let operator = Remembering::holding(&["shell:cargo test"]);
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    let shell = job_shell(ApprovalPolicy::Ask, &operator, true, no_rules()).following(cell.clone());
    let call = json!({ "command": "cargo test -p x" });
    assert_eq!(
        shell.lane(&call),
        Lane::Host(true),
        "the grant pre-answers under ask"
    );

    cell.set(ApprovalPolicy::ReadOnly)
        .expect("the job's cell moves");
    assert_eq!(
        shell.lane(&call),
        Lane::Sandbox(None),
        "read-only serves the sandbox, not the host the grant named"
    );
    let hostless = job_shell(ApprovalPolicy::ReadOnly, &operator, false, no_rules());
    assert_eq!(
        hostless.lane(&call),
        Lane::Refused(READ_ONLY_REFUSAL.to_owned()),
        "with no sandbox to serve it, read-only refuses"
    );
    assert!(
        !hostless.approve(&call).await,
        "and never admits it on the host"
    );

    cell.set(ApprovalPolicy::Never)
        .expect("the cell moves again");
    assert_eq!(
        shell.lane(&call),
        Lane::Refused(NEVER_REFUSAL.to_owned()),
        "never runs no shell, whatever was remembered"
    );
    assert_eq!(operator.asked(), 0, "no mode asks about a grant");
}

/// The store belongs to the job: every shell of it — the root's and a
/// helper's — reads the one store, and another job's shell reads its own.
#[tokio::test]
async fn a_helper_of_the_same_job_sees_the_grant_and_another_job_does_not() {
    let job = Remembering::holding(&["shell:cargo test"]);
    let call = json!({ "command": "cargo test -p y" });
    let root = job_shell(ApprovalPolicy::Ask, &job, true, no_rules());
    let helper = helper_shell(&job);
    assert!(
        root.approve(&call).await && helper.approve(&call).await,
        "the root's and the helper's shell read the job's one store"
    );
    assert_eq!(job.asked(), 0, "neither of them asks");

    let other_job = Remembering::holding(&[]);
    let stranger = helper_shell(&other_job);
    assert_eq!(
        stranger.lane(&call),
        Lane::Host(false),
        "another job remembers nothing of this one"
    );
    assert!(!stranger.approve(&call).await);
    assert_eq!(other_job.asked(), 1, "it asks");
}
