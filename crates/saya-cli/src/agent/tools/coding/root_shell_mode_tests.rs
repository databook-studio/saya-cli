//! The job's one permission-mode cell, read per call: a mode stated between two
//! `approve` calls decides the second one without rebuilding the toolset, and
//! every guard that wins today still wins — a `[permissions]` deny rule in
//! every mode including `bypass`, and `read-only` never asking.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use saya_agent::{ApprovalDecider, ApprovalPolicy};
use saya_config::{CommandRule, CommandRules};
use serde_json::json;

use super::root_shell_tests::fixture::{
    RecordingOperator, TempCheckout, call, host_shell_over_mode_cell, shell_over,
    shell_over_mode_cell,
};
use crate::agent::tools::shell_policy::{ModeCell, definition};

/// A trusted command-prefix table with one deny rule and no allow rule.
fn deny_rules(deny: &[&str]) -> CommandRules {
    CommandRules {
        allow: Vec::new(),
        deny: deny
            .iter()
            .map(|text| CommandRule::parse(text).expect("the test rule parses"))
            .collect(),
        diagnostics: Vec::new(),
    }
}

/// The defect this pins: the shell copied its mode at build, so a mode stated
/// afterwards changed nothing until the toolset was rebuilt. A root shell built
/// under `ask` whose cell moves to `read-only` must answer its second call as a
/// toolset built under `read-only` does.
#[tokio::test]
async fn a_mode_stated_between_two_calls_decides_the_second() {
    let checkout = TempCheckout::new("mode-cell-narrow");
    let operator = RecordingOperator::new(true);
    let mode = ModeCell::new(ApprovalPolicy::Ask);
    let Some(shell) = shell_over_mode_cell(
        checkout.path(),
        mode.clone(),
        Some(operator.clone()),
        CommandRules::default(),
    ) else {
        return;
    };
    let Some(under_read_only) = shell_over(
        checkout.path(),
        ApprovalPolicy::ReadOnly,
        Some(operator.clone()),
    ) else {
        return;
    };
    let definition = definition();

    // Under `ask`, with an operator present, the host call is put to them.
    assert!(shell.approve(&definition, &call("true")).await);
    assert_eq!(operator.calls(), 1, "the asking mode asks");

    mode.set(ApprovalPolicy::ReadOnly)
        .expect("an unfrozen cell takes a change");

    // The same toolset now answers exactly as one composed under `read-only`:
    // the sandbox lane, unasked, with no refusal.
    assert!(shell.approve(&definition, &call("true")).await);
    assert_eq!(operator.calls(), 1, "read-only never asks");
    assert_eq!(
        shell.would_ask(&definition, &call("true")),
        under_read_only.would_ask(&definition, &call("true")),
        "the rebuilt-free toolset asks what a read-only one asks"
    );
    assert_eq!(
        shell.refusal_detail(&definition, &call("true")),
        under_read_only.refusal_detail(&definition, &call("true")),
        "and refuses what a read-only one refuses"
    );
    let ran = shell
        .call("shell", &call("true"))
        .await
        .expect("the sandbox lane runs the call");
    assert_eq!(ran["sandboxed"], true, "the call ran in the sandbox: {ran}");
}

/// A deny rule is decided first in the lane, in every mode: a cell that moves
/// from `read-only` to `bypass` between two calls admits nothing a
/// `[permissions]` rule denies.
#[tokio::test]
async fn a_switch_to_bypass_never_admits_a_denied_command() {
    let checkout = TempCheckout::new("mode-cell-deny");
    let mode = ModeCell::new(ApprovalPolicy::ReadOnly);
    let Some(shell) =
        host_shell_over_mode_cell(checkout.path(), mode.clone(), None, deny_rules(&["rm"]))
    else {
        return;
    };
    let definition = definition();
    let denied = json!({ "command": "rm -rf build", "description": "a probe" });

    assert!(
        !shell.approve(&definition, &call("true")).await,
        "read-only refuses the host lane outright"
    );
    assert!(
        matches!(
            shell.refusal_detail(&definition, &denied),
            Some(reason) if reason.contains("`rm`")
        ),
        "the deny rule decides before the mode's own refusal"
    );

    mode.set(ApprovalPolicy::Bypass)
        .expect("an unfrozen cell takes a change");

    assert!(
        shell.approve(&definition, &call("true")).await,
        "the switch reached the toolset: bypass allows the host"
    );
    assert!(
        !shell.approve(&definition, &denied).await,
        "a deny rule refuses in every mode, bypass included"
    );
    assert!(
        matches!(
            shell.refusal_detail(&definition, &denied),
            Some(reason) if reason.contains("`rm`")
        ),
        "the refusal names the rule that decided it"
    );
}
