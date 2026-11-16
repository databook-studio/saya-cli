//! Shared assembly of the engine's collaborators: the pieces a fresh run and
//! a resume both need, built from config the way `ask` builds a turn's —
//! the provider from the run's `orchestrator` endpoint, the connection
//! registry from the resolved profile, the executor over the database tools.
//!
//! Runs are headless by construction: the approval decider is the frozen
//! session policy (`prompt_approval::TerminalApproval::frozen`) — seeded from
//! the run's `--allow` tokens, unable to prompt, unable to accumulate — so an
//! `ask`-mode call the seeds do not cover denies with the engine's own
//! reason, `never` denies, and the default is read-only — read-shaped tools
//! run, anything needing an interactive decision or an external side effect
//! is denied. The run's scopes (not the per-call decider) are the approval
//! surface for capabilities; the seeds are the per-call grant words the
//! scopes' grammar stated.

use crate::agent::runtime::query_data_allowed_for_endpoint;
use crate::agent::tools::DatabaseTools;
use crate::agent::{profile, provider};
use crate::config::runtime::RuntimeConfig;
use crate::connection;
use crate::prompt_approval::TerminalApproval;
use saya_agent::ChatProvider;
use saya_harness::fetch::{DownloadBudget, ReqwestTransport};
use saya_types::Budgets;
use std::sync::Arc;

use super::mode::RunApproval;
use super::tools::{RunFetch, run_shell};
use crate::agent::tools::coding::shell_secrets::host_policy;
use crate::agent::tools::consent::{Caller, OwnApproval, OwnedToolApproval};
use crate::agent::tools::shell_policy::ShellTool;
use crate::mcp::client::McpTools;

/// How much workspace the episode brief's manifest walks: names, sizes,
/// digests — never bulk contents. Conservative brief bounds, fixed here so
/// every run brief is built the same way; the workspace's own byte and file
/// budgets are a plan-contract concern.
const MANIFEST_MAX_FILES: usize = 512;
const MANIFEST_MAX_FILE_BYTES: u64 = 256 * 1024;

/// The manifest bounds every run brief is built with.
pub(super) fn manifest_bounds() -> saya_harness::engine::ManifestBounds {
    saya_harness::engine::ManifestBounds {
        max_files: MANIFEST_MAX_FILES,
        max_file_bytes: MANIFEST_MAX_FILE_BYTES,
    }
}

/// A step whose budget is unset inherits the run's budgets as its ceilings
/// (the `StepSpec` contract's layering rule). The engine's brief reads only
/// the step budget, so the composition root applies the inheritance when
/// binding: the persisted plan is the layered one a resume replays.
pub(super) fn bind_step_budgets(plan: saya_types::RunPlan, run: &Budgets) -> saya_types::RunPlan {
    let mut plan = plan;
    for step in &mut plan.steps {
        if step.budget.is_none() {
            step.budget = Some(run.clone());
        }
    }
    plan
}

/// Everything the engine's drivers need that the composition root builds
/// once per run or resume: the provider, the shared executor the per-step
/// toolsets wrap, the approval decider, the model the episodes call, and
/// the privacy gate the toolset builder advertises the universe under.
pub(super) struct Pieces {
    pub(super) provider: Box<dyn ChatProvider>,
    pub(super) tools: Arc<DatabaseTools>,
    /// The run-level fetch wiring, built once per run when the run approved
    /// a fetch scope: the shared transport and the download wallet whose
    /// clones arm both the fetch-capable steps' members and the sink's
    /// pause check. `None` means the run did not approve fetch, nothing
    /// fetch-shaped exists, and the sink's download check is inert.
    pub(super) fetch: Option<RunFetch>,
    /// The run's `shell`, built once per run over the run's workspace — the
    /// same tool the coding root advertises, with the same lane decision.
    /// Every step's toolset carries it. `None` where the run has no shell at
    /// all: mode `never`, or `read-only` on a host with no proven sandbox.
    pub(super) shell: Option<Arc<ShellTool>>,
    /// The run's MCP lane: the configured servers' tools, started once per
    /// run under the run's admitted mode. `None` where the mode reaches no
    /// server or the configuration names none.
    pub(super) mcp: Option<Arc<McpTools>>,
    /// The capabilities plan validation must see: the run's approved scopes,
    /// unchanged. A plan asking for a scope `--allow` did not state refuses
    /// as needs-approval, and the removed runner/interpreter families cannot
    /// be stated at all (`scopes/parser.rs`), so no plan can carry one.
    pub(super) plan_scopes: saya_types::Capabilities,
    /// The run's approval decider: the frozen session policy, with a `shell`
    /// call routed to the shell's own lane decision so the command door is
    /// decided exactly as the surface that advertises it decides it.
    pub(super) decider: OwnedToolApproval,
    pub(super) model: String,
    pub(super) profile_names: Vec<String>,
    pub(super) allow_query_data: bool,
}

/// Assembles the pieces. The model is the `orchestrator` endpoint's — the
/// role every run has — resolved over the `[ai]` block exactly the way
/// endpoint resolution layers it. `scopes` are the run's approved
/// capabilities: the run's `shell` is built once per run, fresh and resume
/// alike. `approval` is
/// the run boundary's admitted mode (`mode.rs`): the composition is
/// unreachable in bypass mode, because no entry point can construct the
/// admitted type around it. `allow_tokens` are the run's stated scopes as
/// the grammar's words — the frozen decider's seeds and the journal
/// payload's carried words; a fresh run states them from `--allow`, a resume
/// from the journal. `profile_override` is the host's active connection
/// profile (what a nested child's `--profile` forwards); `None` keeps the
/// resolved default. `cancellation` is the run's token: the shell kills its
/// child's whole process group when the run is stopped. Errors are
/// configuration, connection, or admission problems (exit-code class 3),
/// reported as text for the caller to emit.
// The composition root's inputs are the run's own facts — config, workspace,
// boundary, stated scopes, cancellation — and none of them is derived from
// another; bundling them would only move the same eight names one level down.
#[allow(clippy::too_many_arguments)]
pub(super) async fn assemble(
    runtime: &RuntimeConfig,
    profile_override: Option<&String>,
    scopes: &saya_types::Capabilities,
    workspace: std::sync::Arc<saya_harness::workspace::Workspace>,
    approval: RunApproval,
    allow_tokens: &[String],
    cancellation: &saya_agent::CancellationToken,
) -> Result<Pieces, String> {
    let endpoint = runtime
        .resolved
        .endpoints
        .get(saya_config::ORCHESTRATOR_ROLE)
        .cloned()
        .ok_or_else(|| "no orchestrator endpoint is resolved; check the [ai] config".to_string())?;
    let mut ai = runtime.resolved.ai.clone();
    ai.provider = endpoint.provider;
    ai.model = endpoint.model.clone();
    ai.base_url = endpoint.base_url.clone();
    ai.api_key = endpoint.api_key.clone();
    let provider =
        provider::build(&ai, &runtime.secret_resolver()).map_err(|error| error.to_string())?;
    let (profile_name, profile) =
        profile::selected(runtime, profile_override).map_err(|error| error.to_string())?;
    let (registry, _failures) = match profile.as_ref() {
        Some(primary) => connection::build_registry(
            (&runtime.secret_resolver(), &runtime.cache_scope),
            runtime.resolved.query_timeout_seconds,
            false,
            profile_name.as_deref().unwrap_or(""),
            primary,
            &[],
            None,
        )
        .await
        .map_err(|error| error.to_string())?,
        None => (connection::ConnectionRegistry::new(""), Vec::new()),
    };
    let allow_query_data =
        query_data_allowed_for_endpoint(ai.provider, ai.base_url.as_deref(), ai.allow_data_sharing);
    let profile_names = registry
        .names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    // Shared per run: one registry and one executor the per-step toolsets
    // all wrap. The definitions themselves are built per step by the
    // toolset builder (`tools.rs`), from these same flags.
    let workspace_root = workspace.root().to_path_buf();
    let tools = Arc::new(
        DatabaseTools::with_learning(
            registry,
            runtime.resolved.max_rows,
            allow_query_data,
            None,
            None,
        )
        .with_workspace(Some(Arc::clone(&workspace))),
    );
    // Shared per run, admitted before anything runs (fail closed at start,
    // never mid-flight): one transport and one download wallet when the run
    // approved a fetch scope. The per-step toolsets put them behind the
    // composites of the steps that asked for fetch; a step that did not ask
    // never sees them, and the sink's download check stays inert.
    let fetch = match scopes.fetch {
        Some(_) => {
            let transport = ReqwestTransport::new()
                .map_err(|error| format!("the fetch transport could not be built: {error}"))?;
            Some(RunFetch {
                transport: Arc::new(transport),
                budget: DownloadBudget::default(),
            })
        }
        None => None,
    };
    // Shared per run, built before anything runs (fail closed at start,
    // never mid-flight): the run's one `shell`, composed over the run's
    // workspace with the same secret-stripping host policy the coding root
    // uses. A run has no shell prompt — a headless run cannot ask anyone —
    // so a host call under `ask` falls back to the sandbox when one is
    // proven and is refused with a reason when none is, exactly as the root
    // decides it.
    let shell = run_shell(
        &workspace,
        approval.policy(),
        None,
        host_policy(runtime),
        cancellation.clone(),
        runtime.resolved.permissions.rules.clone(),
    );
    // The run's MCP lane, built once per run under the run's admitted mode.
    // A run has no operator: under `ask` the lane is composed and every call
    // is refused with the reason naming the flag that would reach a server,
    // and `read-only`/`never` compose no lane at all. The lane's lifetime is
    // the run's; its servers close when the run's pieces drop.
    let mcp = McpTools::connect_async(
        runtime.resolved.mcp_servers.clone(),
        Arc::new(runtime.secret_resolver()),
        approval.policy(),
        None,
        Caller::Session,
        cancellation.clone(),
    )
    .await;
    // The run's decider is the frozen session policy, seeded from the
    // run's stated scopes: a `sql:<connection>` token pre-answers
    // the read-shaped SQL calls that name the connection under `ask`
    // mode, and every other ask the seeds do not cover denies with the
    // engine's own reason. The composition facts are the run's own —
    // built from the approved scopes and the admitted members, the same
    // gates the per-step toolsets build from — so a seed pre-answers exactly
    // the calls the composition carries. A `shell` call never reaches
    // these facts: [`OwnedToolApproval`] routes it to the shell's own lane
    // decision first. The primary stays unbound — only a call that names its
    // connection suggests a token — and the policy cannot accumulate: a
    // headless session grant is impossible.
    let mut owners: Vec<Arc<dyn OwnApproval>> = shell
        .iter()
        .map(|shell| Arc::clone(shell) as Arc<dyn OwnApproval>)
        .collect();
    if let Some(mcp) = &mcp {
        owners.push(Arc::clone(mcp) as Arc<dyn OwnApproval>);
    }
    let decider = OwnedToolApproval::new(
        owners,
        Arc::new(TerminalApproval::frozen(
            approval.policy(),
            allow_tokens,
            super::decider_facts::for_frozen_decider(scopes, fetch.as_ref(), &workspace_root),
        )),
    );
    Ok(Pieces {
        provider,
        tools,
        fetch,
        shell,
        mcp,
        plan_scopes: scopes.clone(),
        decider,
        model: ai.model,
        profile_names,
        allow_query_data,
    })
}
