//! The lane decision's truthfulness: a host call under `ask` with no operator
//! to answer it runs in the sandbox and the lane says so, the refusal names the
//! way out, and neither text outgrows what a model reads at a glance.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::ApprovalPolicy;
use saya_config::{CommandRule, CommandRules};
use serde_json::{Value, json};

use super::policy::{HOST_APPROVAL_UNAVAILABLE, Lane, SANDBOX_DOWNGRADE_NOTICE, ShellPolicy};
use crate::agent::tools::consent::{Caller, Operator, Prompt};

/// An operator that answers every call; only its presence matters here.
struct AlwaysApprove;

#[async_trait]
impl Operator for AlwaysApprove {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        true
    }
}

fn prompt() -> Prompt {
    crate::agent::tools::shell_policy::prompt(Arc::new(AlwaysApprove), Caller::Session)
}

/// No rules: every call is decided as if the table were absent.
fn no_rules() -> CommandRules {
    CommandRules::default()
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
    }
}

fn with_rules(
    policy: ApprovalPolicy,
    prompt: Option<Prompt>,
    has_sandbox: bool,
    rules: CommandRules,
) -> ShellPolicy {
    ShellPolicy::new(policy, prompt, has_sandbox, rules, None)
}

/// A host call: `sandbox` is omitted, so the schema default — the host lane —
/// applies. `npm install` is the acceptance's own example.
fn host_call() -> Value {
    json!({ "command": "npm install" })
}

/// A call that names the sandbox lane.
fn sandbox_call() -> Value {
    json!({ "command": "npm install", "sandbox": true })
}

/// The case the reroute exists for: `ask` with no operator and a proven sandbox
/// reroutes the call to the sandbox — and the reroute is recorded as the lane's
/// own reason, so a network failure is attributable instead of invisible.
#[test]
fn ask_with_no_operator_reroutes_to_the_sandbox_and_says_why() {
    let policy = with_rules(ApprovalPolicy::Ask, None, true, no_rules());
    assert_eq!(
        policy.lane(&host_call()),
        Lane::Sandbox(Some(SANDBOX_DOWNGRADE_NOTICE)),
        "a rerouted host call carries the reason it was rerouted"
    );
    assert!(
        policy.refusal_detail(&host_call()).is_none(),
        "a rerouted call is not refused: it ran, in the sandbox"
    );
}

/// Nothing rerouted carries no notice: the mode allowed the host, or the call
/// asked for the sandbox itself.
#[test]
fn a_host_call_that_needed_no_operator_carries_no_notice() {
    assert_eq!(
        with_rules(ApprovalPolicy::Bypass, None, true, no_rules()).lane(&host_call()),
        Lane::Host(false),
        "bypass allows the host outright"
    );
    assert_eq!(
        with_rules(ApprovalPolicy::Ask, Some(prompt()), true, no_rules()).lane(&host_call()),
        Lane::Host(false),
        "an operator is there to answer"
    );
    assert_eq!(
        with_rules(ApprovalPolicy::Ask, None, true, no_rules()).lane(&sandbox_call()),
        Lane::Sandbox(None),
        "the confinement was the call's own choice"
    );
}

/// An operator that refuses every call: a rule-allowed call must not reach it.
struct AlwaysRefuse;

#[async_trait]
impl Operator for AlwaysRefuse {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        false
    }
}

fn refusing_prompt() -> Prompt {
    crate::agent::tools::shell_policy::prompt(Arc::new(AlwaysRefuse), Caller::Session)
}

/// A rule-allowed host call under `ask` takes the host lane without an
/// operator, and the operator is not consulted even when it would refuse.
#[tokio::test]
async fn an_allowed_prefix_reaches_the_host_without_asking() {
    let policy = with_rules(
        ApprovalPolicy::Ask,
        Some(refusing_prompt()),
        true,
        rules(&["npm install"], &[]),
    );
    assert_eq!(
        policy.lane(&host_call()),
        Lane::Host(true),
        "the stated prefix is the answer"
    );
    assert!(
        policy.approve(&host_call()).await,
        "a rule-allowed call never reaches the operator"
    );
    // Headless is the case the rule exists for: no operator, still the host.
    let headless = with_rules(
        ApprovalPolicy::Ask,
        None,
        true,
        rules(&["npm install"], &[]),
    );
    assert_eq!(headless.lane(&host_call()), Lane::Host(true));
    assert!(headless.approve(&host_call()).await);
}

/// A prefix covers whole tokens only, and a compound command needs every
/// segment covered: `npm install && rm -rf build` is not `npm install`.
#[test]
fn an_allow_rule_matches_whole_tokens_and_every_segment() {
    let policy = with_rules(
        ApprovalPolicy::Ask,
        None,
        true,
        rules(&["npm install"], &[]),
    );
    assert_eq!(
        policy.lane(&json!({"command": "npm install --save left-pad"})),
        Lane::Host(true)
    );
    assert_eq!(
        policy.lane(&json!({"command": "npm installx"})),
        Lane::Sandbox(Some(SANDBOX_DOWNGRADE_NOTICE))
    );
    assert_eq!(
        policy.lane(&json!({"command": "npm install && rm -rf build"})),
        Lane::Sandbox(Some(SANDBOX_DOWNGRADE_NOTICE))
    );
}

/// Substitution and heredocs are never auto-allowed: the matcher cannot see
/// what they run, so the call takes the ordinary lane.
#[test]
fn a_substitution_is_never_auto_allowed() {
    let policy = with_rules(
        ApprovalPolicy::Ask,
        None,
        true,
        rules(&["npm install"], &[]),
    );
    assert_eq!(
        policy.lane(&json!({"command": "npm install $(curl -s https://x)"})),
        Lane::Sandbox(Some(SANDBOX_DOWNGRADE_NOTICE))
    );
}

/// A deny rule refuses in every mode, `bypass` included, and wins over an
/// allow rule for the same prefix. The refusal names the rule.
#[test]
fn a_deny_rule_refuses_every_mode_and_wins_over_allow() {
    for policy in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
    ] {
        let shell = with_rules(policy, Some(prompt()), true, rules(&["rm"], &["rm"]));
        let lane = shell.lane(&json!({"command": "rm -rf build"}));
        assert!(
            matches!(&lane, Lane::Refused(reason) if reason.contains("`rm`")),
            "{policy:?} must refuse a denied command and name the rule: {lane:?}"
        );
        assert!(
            shell
                .refusal_detail(&json!({"command": "rm -rf build"}))
                .is_some(),
            "{policy:?} must report the typed refusal"
        );
    }
    // Even a sandboxed call: the rule is not about the lane.
    let sandboxed = with_rules(ApprovalPolicy::Bypass, None, true, rules(&[], &["rm"]));
    assert!(matches!(
        sandboxed.lane(&json!({"command": "rm -rf build", "sandbox": true})),
        Lane::Refused(_)
    ));
}

/// An allow rule never widens the modes that withhold the host: `read-only`
/// keeps the sandbox, `never` keeps its refusal.
#[test]
fn an_allow_rule_does_not_widen_read_only_or_never() {
    let read_only = with_rules(
        ApprovalPolicy::ReadOnly,
        None,
        true,
        rules(&["npm install"], &[]),
    );
    assert_eq!(read_only.lane(&host_call()), Lane::Sandbox(None));
    let never = with_rules(
        ApprovalPolicy::Never,
        None,
        true,
        rules(&["npm install"], &[]),
    );
    assert!(matches!(never.lane(&host_call()), Lane::Refused(_)));
}

/// The refusal and the notice both name the one flag that gets a headless
/// driver onto the host, and both stay short enough to read.
#[test]
fn the_notice_and_the_refusal_name_the_way_to_the_host() {
    for text in [SANDBOX_DOWNGRADE_NOTICE, HOST_APPROVAL_UNAVAILABLE] {
        assert!(
            text.contains("--approval-mode bypass"),
            "the way to host execution is named: {text}"
        );
        assert!(
            text.contains("--allow-command <prefix>"),
            "the rule flag that reaches the host is named: {text}"
        );
        assert!(
            text.len() <= 200,
            "the model-facing text stays short ({} bytes): {text}",
            text.len()
        );
    }
    assert!(
        SANDBOX_DOWNGRADE_NOTICE.contains("no network"),
        "the sandbox's own cost is named: {SANDBOX_DOWNGRADE_NOTICE}"
    );
    assert!(
        SANDBOX_DOWNGRADE_NOTICE.contains("sandbox"),
        "where the call ran is named: {SANDBOX_DOWNGRADE_NOTICE}"
    );
}
