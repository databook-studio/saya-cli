//! The headless `ask` lane, stated truthfully: with no operator to answer
//! it, a host call runs in the proven sandbox and its result says so; with no
//! sandbox to fall back to it is refused, with a reason naming the way out; and
//! a call nothing was taken from carries no notice.

use saya_agent::{ApprovalDecider, ApprovalPolicy};
use saya_harness::runner::HostPolicy;

use super::definition;
use super::fixture::{
    NO_OPERATOR_REFUSAL, TempCheckout, call, host_shell_over, sandbox_call, shell_over,
};
use crate::agent::tools::shell_policy::SANDBOX_DOWNGRADE_NOTICE;

/// With no operator to answer — a headless run — a host call falls back to the
/// sandbox when one is proven: approved, executed sandboxed, and told so. The
/// result carries the reason, so the model never believes the command reached
/// the host: apart from the `sandboxed` flag the reroute would be invisible.
#[tokio::test]
async fn a_headless_call_falls_back_to_the_sandbox_when_one_is_proven() {
    let checkout = TempCheckout::new("headless-fallback");
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, None) else {
        return;
    };
    assert!(shell.approve(&definition(), &call("true")).await);
    assert!(shell.refusal_detail(&definition(), &call("true")).is_none());
    let ran = shell
        .call("shell", &call("true"))
        .await
        .expect("the call runs in the sandbox");
    assert_eq!(
        ran["sandboxed"], true,
        "the headless host call ran sandboxed: {ran}"
    );
    assert_eq!(
        ran["lane_notice"].as_str(),
        Some(SANDBOX_DOWNGRADE_NOTICE),
        "the reroute is stated in the result: {ran}"
    );
}

/// A call the mode allowed on the host, and a call that asked for the sandbox
/// itself, carry no notice: there was no reroute to explain.
#[tokio::test]
async fn a_host_call_that_needed_no_operator_carries_no_notice() {
    let checkout = TempCheckout::new("no-notice");

    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Bypass, None) else {
        return;
    };
    let host = shell
        .call("shell", &call("true"))
        .await
        .expect("bypass runs the call on the host");
    assert_eq!(host["sandboxed"], false, "bypass runs on the host: {host}");
    assert!(
        host.get("lane_notice").is_none(),
        "an allowed host call explains nothing: {host}"
    );

    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, None) else {
        return;
    };
    let sandboxed = shell
        .call("shell", &sandbox_call("true"))
        .await
        .expect("the call runs in the sandbox");
    assert_eq!(sandboxed["sandboxed"], true);
    assert!(
        sandboxed.get("lane_notice").is_none(),
        "the call asked for the sandbox: {sandboxed}"
    );
}

/// With no operator and no sandbox lane there is nothing to fall back to: the
/// call is refused, with the reason the refusal names.
#[tokio::test]
async fn a_host_call_with_no_operator_and_no_sandbox_is_refused() {
    let checkout = TempCheckout::new("host-no-operator");
    let Some(shell) = host_shell_over(
        checkout.path(),
        ApprovalPolicy::Ask,
        None,
        HostPolicy::default(),
    ) else {
        return;
    };
    assert!(!shell.approve(&definition(), &call("true")).await);
    assert_eq!(
        shell
            .refusal_detail(&definition(), &call("true"))
            .as_deref(),
        Some(NO_OPERATOR_REFUSAL)
    );
}
