//! Tests for `CliChildFactory::build`: one child's provider, toolset,
//! definitions and approval composed from ONE immutable authority view. The
//! factory is refused when the view grants a command scope without a proven
//! runner, when the view belongs to another checkout, and a runner present
//! without a view scope is simply ignored. Authored, not executed: saya-cli
//! depends on DuckDB and is never built in this environment.

use std::error::Error as _;
use std::{fs, path::Path, sync::Arc, time::Duration};

use super::tests::{Sandbox, checkout, implementer_view};
use super::{ChildToolsError, CliChildFactory};
use saya_agent::{AgentLimits, AgentRequest, AnsweringLimits, CancellationToken};
use saya_config::{AiProvider, ResolvedAi};
use saya_harness::coding::child::{ChildExecutionSpec, ChildFactory};
use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildAuthorityView, ChildRole, CommandCheckoutProfile, PlatformSupport,
    RootDelegationInput, RootDelegationScope,
};
use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
use saya_harness::runner::{RunProgram, StaticCredentialSource};
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, RepositoryId, RunnerScope,
    Sha256Digest, SnapshotId, WorkspacePath,
};

fn path(value: &str) -> WorkspacePath {
    WorkspacePath::try_from(value.to_owned()).unwrap()
}

/// An Implementer narrowed to exactly `["a/ok.txt"]` whose capabilities grant
/// the runner exactly `programs`: both the root input and the request
/// capabilities carry the same runner scope, on a platform with an isolated
/// private command checkout.
fn runner_view(checkout: &ManagedCheckoutId, programs: &[&str]) -> ChildAuthorityView {
    let scope = || RunnerScope::new(programs.iter().map(|p| p.to_string()).collect()).unwrap();
    let write_paths = || vec![path("a/ok.txt")];
    let mut root_capabilities = Capabilities::default();
    root_capabilities.workspace_write = true;
    root_capabilities.runner = Some(scope());
    let mut request_capabilities = root_capabilities.clone();
    request_capabilities.runner = Some(scope());
    RootDelegationScope::new(RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repository").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: Some(checkout.clone()),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        write_paths: write_paths(),
        capabilities: root_capabilities,
        database_profiles: Vec::new(),
    })
    .unwrap()
    .narrow(
        ChildAuthorityRequest::new(
            ChildRole::Implementer,
            write_paths(),
            request_capabilities,
            Vec::new(),
        )
        .unwrap(),
        PlatformSupport {
            workspace_edits: true,
            command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
            network_fetch: true,
            database_reads: true,
        },
    )
    .unwrap()
}

/// A real runner proven on this host against the sandbox root, recording
/// outside it. `None` when the host probe refuses the sandbox: an accepted
/// skip.
fn real_runner(root: &Path, records: &Path) -> Option<Arc<RunProgram>> {
    let sandbox = RunSandbox::new([root.to_path_buf()], std::iter::empty()).unwrap();
    match sandbox.prepare(Path::new("/bin")).unwrap() {
        SandboxProvision::Proven { spawn, .. } => {
            fs::create_dir_all(records).expect("records directory must create");
            Some(Arc::new(
                RunProgram::for_step(
                    spawn,
                    Some(RunnerScope::new(vec!["pwd".into(), "ls".into()]).unwrap()),
                    None,
                    Duration::from_secs(10),
                    Arc::new(StaticCredentialSource::new([])),
                )
                .with_record_dir(records.to_path_buf()),
            ))
        }
        SandboxProvision::Refused(_) => None,
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "inspect the assigned source".to_owned(),
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
        },
        AnsweringLimits {
            max_requests: Some(4),
            max_tool_calls: Some(64),
            max_reported_tokens: Some(10_000),
        },
    )
}

fn spec(view: ChildAuthorityView) -> ChildExecutionSpec {
    let (agent_limits, answering_limits) = limits();
    ChildExecutionSpec::new(view, request(), agent_limits, answering_limits, None).unwrap()
}

fn test_ai() -> ResolvedAi {
    ResolvedAi {
        provider: AiProvider::Ollama,
        model: "test-model".into(),
        base_url: None,
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        max_output_tokens: 4096,
        max_output_tokens_is_default: true,
        context_byte_budget: 64 * 1024,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: vec![250, 500, 1000],
    }
}

fn factory(
    sandbox: &Sandbox,
    checkout_id: &ManagedCheckoutId,
    runner: Option<Arc<RunProgram>>,
) -> CliChildFactory {
    CliChildFactory::new(
        sandbox.pair(checkout_id),
        runner,
        test_ai(),
        Arc::new(saya_config::MapSecretResolver::new([])),
    )
}

// authored, not executed
#[test]
fn factory_builds_for_a_workspace_only_child() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id, &["a/ok.txt"]);
    let sandbox = Sandbox::new("factory-workspace-only");
    let factory = factory(&sandbox, &checkout_id, None);

    assert!(
        factory
            .build(&spec(view), &CancellationToken::new())
            .is_ok()
    );
}

// authored, not executed
#[test]
fn factory_refuses_a_command_scope_without_a_proven_runner() {
    let checkout_id = checkout("checkout");
    let view = runner_view(&checkout_id, &["pwd"]);
    let sandbox = Sandbox::new("factory-no-runner");
    let factory = factory(&sandbox, &checkout_id, None);

    let err = factory
        .build(&spec(view), &CancellationToken::new())
        .err()
        .expect("a command scope without a proven runner must refuse");
    let source = err
        .source()
        .and_then(|s| s.downcast_ref::<ChildToolsError>());
    assert!(matches!(source, Some(ChildToolsError::ConfinementMissing)));
}

// authored, not executed
#[test]
fn factory_refuses_a_view_for_another_checkout() {
    let checkout_a = checkout("checkout");
    let checkout_b = checkout("other-checkout");
    let view = implementer_view(&checkout_b, &["a/ok.txt"]);
    let sandbox = Sandbox::new("factory-mismatch");
    let factory = factory(&sandbox, &checkout_a, None);

    let err = factory
        .build(&spec(view), &CancellationToken::new())
        .err()
        .expect("a view for another checkout must refuse");
    let source = err
        .source()
        .and_then(|s| s.downcast_ref::<ChildToolsError>());
    assert!(matches!(source, Some(ChildToolsError::CheckoutMismatch)));
}

// authored, not executed
#[test]
fn factory_ignores_a_runner_when_the_view_grants_no_command_scope() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id, &["a/ok.txt"]);
    let sandbox = Sandbox::new("factory-runner-ignored");
    let Some(runner) = real_runner(&sandbox.ws_root(), &sandbox.outer.join("records")) else {
        return;
    };
    let factory = factory(&sandbox, &checkout_id, Some(runner));

    assert!(
        factory
            .build(&spec(view), &CancellationToken::new())
            .is_ok()
    );
}
