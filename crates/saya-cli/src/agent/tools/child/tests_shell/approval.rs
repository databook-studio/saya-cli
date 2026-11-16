//! A helper's host call under `ask` and `bypass`, and the child approval that
//! routes `shell` to the shell's own lane.

use std::sync::Arc;

use saya_agent::{ApprovalDecider, ApprovalPolicy};
use serde_json::json;

use super::super::tests::{Sandbox, checkout, implementer_view};
use super::helper_shell;
use super::support::{HELPER_LINE, RecordingOperator, call};
use crate::agent::tools::shell_policy;

/// Under `ask` with an operator present, a helper's call that names no sandbox
/// is a host call: the operator is asked exactly once, under the line that
/// names the helper, and the answer decides.
#[tokio::test]
async fn ask_puts_a_helper_host_call_to_the_operator_under_the_helper_line() {
    let sandbox = Sandbox::new("helper-ask");
    let approving = RecordingOperator::new(true);
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Ask, Some(approving.clone())) else {
        return;
    };
    assert!(
        shell
            .approve(&shell_policy::definition(), &call("true"))
            .await
    );
    assert_eq!(approving.calls(), 1, "a host call asks exactly once");
    assert_eq!(
        approving.subjects(),
        vec![HELPER_LINE.to_owned()],
        "the operator's prompt names the helper and its job"
    );
    let ran = shell
        .run_call(&call("true"))
        .await
        .expect("the approved host call runs");
    assert_eq!(ran["sandboxed"], false, "the call ran on the host: {ran}");

    let refusing = RecordingOperator::new(false);
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Ask, Some(refusing.clone())) else {
        return;
    };
    assert!(
        !shell
            .approve(&shell_policy::definition(), &call("true"))
            .await
    );
    assert_eq!(refusing.calls(), 1);
}

/// `bypass` runs a helper's call that names no sandbox on the host and never
/// asks.
#[tokio::test]
async fn bypass_runs_a_helper_host_call_without_asking() {
    let sandbox = Sandbox::new("helper-bypass");
    let operator = RecordingOperator::new(false);
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Bypass, Some(operator.clone())) else {
        return;
    };
    assert!(
        shell
            .approve(&shell_policy::definition(), &call("true"))
            .await
    );
    assert_eq!(operator.calls(), 0, "bypass never asks");
    let ran = shell
        .run_call(&call("true"))
        .await
        .expect("the host call runs");
    assert_eq!(ran["sandboxed"], false, "bypass runs on the host: {ran}");
}

/// The child approval the factory installs routes `shell` to the shell's own
/// lane decision while every other call keeps the child's exact-write answer.
#[tokio::test]
async fn the_child_approval_routes_shell_to_the_shell_lane() {
    let sandbox = Sandbox::new("helper-approval");
    let operator = RecordingOperator::new(false);
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Ask, Some(operator.clone())) else {
        return;
    };
    let inner = saya_harness::coding::policy::ChildApprovalDecider::from_view(&implementer_view(
        &checkout("helper-approval"),
    ));
    let approval = crate::agent::tools::consent::OwnedToolApproval::new(
        vec![Arc::clone(&shell) as Arc<dyn crate::agent::tools::consent::OwnApproval>],
        Arc::new(inner),
    );
    let definition = shell_policy::definition();
    // The shell's own lane decides: the operator refuses, so the call is
    // denied even though the child's coarse gate admits `shell`.
    assert!(!approval.approve(&definition, &call("true")).await);
    assert_eq!(operator.calls(), 1, "the operator was asked, not the gate");
    // A sandboxed call never asks where the host proves a sandbox. Where it
    // does not (Linux today), the composed shell is host-only, so the call is
    // a host call and the operator decides it — here, refuses it.
    let sandboxed = json!({ "command": "true", "sandbox": true });
    if shell_policy::prove(&sandbox.ws_root(), ApprovalPolicy::Ask).is_some() {
        assert!(approval.approve(&definition, &sandboxed).await);
        assert_eq!(operator.calls(), 1, "the confinement is the consent");
    } else {
        assert!(
            !approval.approve(&definition, &sandboxed).await,
            "without a sandbox lane the call is a host call the operator refuses"
        );
        assert_eq!(
            operator.calls(),
            2,
            "the operator was asked for the host call"
        );
    }
}

/// The wrapper routes every name of the family, not only the word `shell`: a
/// reader that reaches it is answered by the shell that would serve it, not by
/// the child's coarse gate. A helper's shell owns its background runs, so a
/// reader is admitted without asking — it reaches only a run the helper
/// started — and the operator is never consulted for one.
#[tokio::test]
async fn the_child_approval_routes_every_name_of_the_family_to_the_shell() {
    let sandbox = Sandbox::new("helper-approval-family");
    let operator = RecordingOperator::new(false);
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Ask, Some(operator.clone())) else {
        return;
    };
    let inner = saya_harness::coding::policy::ChildApprovalDecider::from_view(&implementer_view(
        &checkout("helper-approval-family"),
    ));
    let approval = crate::agent::tools::consent::OwnedToolApproval::new(
        vec![Arc::clone(&shell) as Arc<dyn crate::agent::tools::consent::OwnApproval>],
        Arc::new(inner),
    );
    let read = shell_policy::background_definitions(shell_policy::BackgroundOwner::Helper)
        .into_iter()
        .find(|definition| definition.name == "shell_output")
        .expect("the family advertises its reader");
    let call = json!({ "id": 1 });
    assert!(
        approval.approve(&read, &call).await,
        "a helper's shell owns its runs, so a reader carries no new authority"
    );
    assert_eq!(operator.calls(), 0, "a reader never reaches the operator");
    assert!(
        approval.refusal_detail(&read, &call).is_none(),
        "nothing refuses a reader the shell serves"
    );
}
