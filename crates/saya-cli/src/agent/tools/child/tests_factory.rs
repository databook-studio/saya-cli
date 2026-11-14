//! Tests for `CliChildFactory::build`: one child's provider, toolset,
//! definitions and approval composed from ONE immutable authority view. The
//! factory refuses a view that belongs to another checkout. Authored, not
//! executed in this environment.

use std::error::Error as _;
use std::fs;
use std::sync::Arc;

use super::tests::{Sandbox, checkout, implementer_view};
use super::{ChildToolsError, CliChildFactory};
use crate::agent::hooks::HookCaller;
use crate::agent::hooks::test_support::blocking_hooks;
use saya_agent::{AgentLimits, AgentRequest, AnsweringLimits, CancellationToken, ToolError};
use saya_config::{AiProvider, ResolvedAi};
use saya_harness::coding::child::{ChildExecutionSpec, ChildFactory};
use saya_harness::coding::policy::ChildAuthorityView;
use saya_types::ManagedCheckoutId;

fn request_with(prompt: &str) -> AgentRequest {
    AgentRequest {
        prompt: prompt.to_owned(),
        profile_names: Vec::new(),
        model: "test-model".to_owned(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn limits() -> (AgentLimits, AnsweringLimits) {
    (
        AgentLimits {
            max_turns: Some(4),
            max_tool_calls: Some(64),
            max_continuations: Some(1),
            permit_candidate_writes: false,
            permit_workspace_writes: false,
            permit_external_effects: false,
            context_byte_budget: 64 * 1024,
            progress: None,
        },
        AnsweringLimits {
            max_requests: Some(4),
            max_tool_calls: Some(64),
            max_reported_tokens: Some(10_000),
        },
    )
}

fn spec(view: ChildAuthorityView) -> ChildExecutionSpec {
    spec_with_prompt(view, "inspect the assigned source")
}

pub(super) fn spec_with_prompt(view: ChildAuthorityView, prompt: &str) -> ChildExecutionSpec {
    let (agent_limits, answering_limits) = limits();
    ChildExecutionSpec::new(
        view,
        request_with(prompt),
        agent_limits,
        answering_limits,
        None,
    )
    .unwrap()
}

pub(super) fn test_ai() -> ResolvedAi {
    ResolvedAi {
        app_attribution: true,
        provider: AiProvider::Ollama,
        model: "test-model".into(),
        base_url: None,
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        request_timeout_seconds: 180,
        max_output_tokens: None,
        reasoning_effort: None,
        context_byte_budget: 64 * 1024,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        compaction_trigger_percent: 50,
        retry_delays_ms: vec![250, 500, 1000],
    }
}

fn factory(sandbox: &Sandbox, checkout_id: &ManagedCheckoutId) -> CliChildFactory {
    CliChildFactory::new(
        sandbox.pair(checkout_id),
        test_ai(),
        Arc::new(saya_config::MapSecretResolver::new([])),
    )
}

// authored, not executed
#[test]
fn factory_builds_for_a_workspace_only_child() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("factory-workspace-only");
    let factory = factory(&sandbox, &checkout_id);

    assert!(
        factory
            .build(&spec(view), &CancellationToken::new())
            .is_ok()
    );
}

/// A `PreToolUse` hook covers a helper's calls, not only the root's: the
/// helper's dispatch path is the job's, so the same engine sits in front of it
/// and the helper's model reads the hook's own reason.
#[tokio::test]
async fn a_pre_tool_use_hook_blocks_a_helpers_call() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("factory-hooks");
    let reason = "no reads from helpers today";
    let factory = factory(&sandbox, &checkout_id).with_hooks(Some(blocking_hooks(
        reason,
        &sandbox.ws_root(),
        HookCaller::Helper {
            agent_id: "agent".to_owned(),
            job_id: "job".to_owned(),
        },
    )));

    let components = factory
        .build(&spec(view), &CancellationToken::new())
        .expect("the child composes");
    let error = components
        .tools()
        .execute("read", serde_json::json!({"path": "a.txt"}))
        .await
        .expect_err("the hook must block this call");
    assert!(
        error.to_string().contains(reason),
        "the helper's model must read the hook's own reason: {error}"
    );
}

/// A write-capable helper is offered the notebook edit only where its
/// checkout actually holds a notebook: with none the definition is dropped
/// from the helper's list — the model pays no context for it — and a call to
/// it is refused as an unknown tool; with one it is advertised, last, where it
/// already stood.
#[tokio::test]
async fn a_helper_offers_the_notebook_edit_only_where_the_checkout_holds_one() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("factory-notebook");
    let factory = factory(&sandbox, &checkout_id);

    let without = factory
        .build(&spec(view.clone()), &CancellationToken::new())
        .expect("the child composes");
    let names: Vec<&str> = without
        .definitions()
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    assert!(
        !names.contains(&"notebook_edit"),
        "a checkout with no notebook offers the helper no notebook edit: {names:?}"
    );
    let refused = without
        .tools()
        .execute(
            "notebook_edit",
            serde_json::json!({"path": "a/ok.ipynb", "mode": "delete", "index": 0}),
        )
        .await
        .expect_err("the dropped name must refuse, not be served");
    assert!(
        matches!(refused, ToolError::UnsupportedTool),
        "the refusal is the unknown-tool one: {refused:?}"
    );

    fs::write(sandbox.ws_root().join("analysis.ipynb"), b"{\"cells\":[]}").unwrap();
    let with = factory
        .build(&spec(view), &CancellationToken::new())
        .expect("the child composes");
    let names: Vec<&str> = with
        .definitions()
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    assert_eq!(
        names.last().copied(),
        Some("notebook_edit"),
        "a notebook in the checkout puts the edit back, last: {names:?}"
    );
}

// authored, not executed
#[test]
fn factory_refuses_a_view_for_another_checkout() {
    let checkout_a = checkout("checkout");
    let checkout_b = checkout("other-checkout");
    let view = implementer_view(&checkout_b);
    let sandbox = Sandbox::new("factory-mismatch");
    let factory = factory(&sandbox, &checkout_a);

    let err = factory
        .build(&spec(view), &CancellationToken::new())
        .err()
        .expect("a view for another checkout must refuse");
    let source = err
        .source()
        .and_then(|s| s.downcast_ref::<ChildToolsError>());
    assert!(matches!(source, Some(ChildToolsError::CheckoutMismatch)));
}
