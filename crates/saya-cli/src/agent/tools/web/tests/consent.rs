//! The consent rule for a fetch: `ask` puts the URL to the operator once per call
//! and honours the answer, `bypass` fetches without asking, a headless `ask`
//! is refused with the reason naming the flag that would reach the network, and
//! the two no-egress modes advertise nothing.

use std::sync::Arc;

use saya_agent::{ApprovalDecider, ApprovalPolicy, ToolError};
use saya_types::AgentId;

use super::super::definition::definition;
use super::super::fixture::{self, CannedNet, RecordingOperator, call, url};
use super::super::tool::{NEVER_REFUSAL, READ_ONLY_REFUSAL, WEB_APPROVAL_UNAVAILABLE, line};
use super::super::{compose, prompt};
use crate::agent::tools::consent::{Caller, Operator};

/// `ask` with an operator present: the URL is put to it exactly once per call,
/// and the answer decides — an approval fetches, a refusal does not.
#[tokio::test]
async fn ask_puts_each_call_to_the_operator_once() {
    let approving = RecordingOperator::new(true);
    let net = Arc::new(CannedNet::serving("hello"));
    let tool = fixture::tool(net, ApprovalPolicy::Ask, Some(approving.clone()))
        .expect("ask advertises the tool");
    assert!(tool.approve(&definition(), &call(&url("/a"))).await);
    assert_eq!(approving.calls(), 1, "one call, one prompt");
    assert!(
        tool.refusal_detail(&definition(), &call(&url("/a")))
            .is_none()
    );
    let fetched = tool
        .call("web_fetch", &call(&url("/a")))
        .await
        .expect("the approved call fetches");
    assert_eq!(fetched["status"], 200);

    // The second call is asked again: no answer is remembered between calls.
    assert!(tool.approve(&definition(), &call(&url("/b"))).await);
    assert_eq!(approving.calls(), 2);

    let refusing = RecordingOperator::new(false);
    let tool = fixture::tool(
        Arc::new(CannedNet::serving("hello")),
        ApprovalPolicy::Ask,
        Some(refusing.clone()),
    )
    .unwrap();
    assert!(!tool.approve(&definition(), &call(&url("/a"))).await);
    assert_eq!(refusing.calls(), 1);
}

/// A headless `ask` job has nobody to consent per fetch, so the call is refused
/// with the reason naming the flag that would reach the network — the same
/// actionable shape the shell's own refusal has.
#[tokio::test]
async fn a_headless_ask_refuses_and_names_the_flag_that_would_fetch() {
    let tool = fixture::tool(
        Arc::new(CannedNet::serving("hello")),
        ApprovalPolicy::Ask,
        None,
    )
    .expect("a headless ask still advertises the tool, so the refusal has somewhere to be read");
    assert!(!tool.approve(&definition(), &call(&url("/a"))).await);
    assert_eq!(
        tool.refusal_detail(&definition(), &call(&url("/a"))),
        Some(WEB_APPROVAL_UNAVAILABLE.to_owned())
    );
    assert!(
        WEB_APPROVAL_UNAVAILABLE.contains("--approval-mode bypass"),
        "the refusal names the way out: {WEB_APPROVAL_UNAVAILABLE}"
    );
    let refused = tool.call("web_fetch", &call(&url("/a"))).await;
    assert!(
        matches!(&refused, Err(ToolError::Fetch(detail)) if detail == WEB_APPROVAL_UNAVAILABLE),
        "a call that arrives anyway is refused with the same reason: {refused:?}"
    );
}

/// `bypass` fetches without asking anyone: the mode is the consent.
#[tokio::test]
async fn bypass_fetches_without_asking() {
    let operator = RecordingOperator::new(false);
    let tool = fixture::tool(
        Arc::new(CannedNet::serving("hello")),
        ApprovalPolicy::Bypass,
        Some(operator.clone()),
    )
    .expect("bypass advertises the tool");
    assert!(tool.approve(&definition(), &call(&url("/a"))).await);
    assert_eq!(operator.calls(), 0, "the mode said so; nobody is asked");
    assert!(tool.call("web_fetch", &call(&url("/a"))).await.is_ok());
}

/// The two no-egress modes advertise nothing: there is no tool to call, so no
/// call can be refused at run time. The reasons below are what the total
/// decision states for them, and each names the mode it belongs to.
#[tokio::test]
async fn the_no_egress_modes_advertise_nothing() {
    for policy in [ApprovalPolicy::ReadOnly, ApprovalPolicy::Never] {
        assert!(
            compose(
                fixture::client(Arc::new(CannedNet::serving("hello"))),
                policy,
                None,
                saya_agent::CancellationToken::new(),
            )
            .is_none(),
            "{policy:?} advertises no fetch"
        );
    }
    assert!(
        READ_ONLY_REFUSAL.contains("read-only"),
        "{READ_ONLY_REFUSAL}"
    );
    assert!(NEVER_REFUSAL.contains("no web page"), "{NEVER_REFUSAL}");
}

/// The prompt's line names the caller: the root names its job, a helper names
/// itself and the job it works for, and the session names no job at all.
#[test]
fn the_prompt_names_who_is_asking() {
    let root = Caller::Root {
        job_id: saya_types::JobId::parse("job-7").unwrap(),
    };
    assert_eq!(line(&root), "Fetch a web page for job job-7");
    let helper = Caller::Helper {
        agent_id: AgentId::parse("helper-2").unwrap(),
        job_id: saya_types::JobId::parse("job-7").unwrap(),
    };
    assert_eq!(
        line(&helper),
        "Helper helper-2 wants to fetch a web page for job job-7"
    );
    assert_eq!(line(&Caller::Session), "Fetch a web page");
}

/// The line a prompt carries is the one its operator is shown, not a second
/// wording built at the call site.
#[tokio::test]
async fn a_prompt_shows_the_line_it_was_built_with() {
    struct Shown(std::sync::Mutex<Option<String>>);
    #[async_trait::async_trait]
    impl Operator for Shown {
        async fn ask(&self, subject: &str, _arguments: &serde_json::Value) -> bool {
            *self.0.lock().unwrap() = Some(subject.to_owned());
            true
        }
    }
    let shown = Arc::new(Shown(std::sync::Mutex::new(None)));
    let built = prompt(
        shown.clone(),
        Caller::Root {
            job_id: saya_types::JobId::parse("job-9").unwrap(),
        },
    );
    assert!(built.ask(&call(&url("/a"))).await);
    assert_eq!(
        shown.0.lock().unwrap().as_deref(),
        Some("Fetch a web page for job job-9")
    );
}
