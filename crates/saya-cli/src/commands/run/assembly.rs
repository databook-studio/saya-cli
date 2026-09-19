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
use saya_harness::scratch::ScratchSql;
use saya_types::Budgets;
use std::sync::Arc;
use std::time::Duration;

use super::mode::RunApproval;
use super::runner::{RunRunner, build as build_runner};
use super::tools::RunFetch;

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
    /// The run's scratch database (ADR 0003), admitted once per run when the
    /// run approved `scratch` — one file shared across every step toolset;
    /// `None` opens nothing and nothing else may open the file.
    pub(super) scratch: Option<Arc<ScratchSql>>,
    /// The run-level fetch wiring, built once per run when the run approved
    /// a fetch scope: the shared transport and the download wallet whose
    /// clones arm both the fetch-capable steps' members and the sink's
    /// pause check. `None` means the run did not approve fetch, nothing
    /// fetch-shaped exists, and the sink's download check is inert.
    pub(super) fetch: Option<RunFetch>,
    /// The run's runner wiring, built once per run when the run approved a
    /// runner scope **and** the startup probe proved the host: the placement
    /// guard, the probe, and the admission check all ran here. `None` when
    /// the run did not approve runner or the host did not prove — either way
    /// the capability is absent from every toolset, never degraded.
    pub(super) runner: Option<RunRunner>,
    /// The capabilities plan validation must see: the run's approved scopes
    /// passed through the provision's own fail-closed rule
    /// (`provision.plan_capabilities(&scopes)`), so a plan asking for the
    /// runner on an unproven host refuses as needs-approval — the honest
    /// refusal — instead of reaching a tool that does not exist.
    pub(super) plan_scopes: saya_types::Capabilities,
    pub(super) decider: TerminalApproval,
    pub(super) model: String,
    pub(super) profile_names: Vec<String>,
    pub(super) allow_query_data: bool,
}

/// Assembles the pieces. The model is the `orchestrator` endpoint's — the
/// role every run has — resolved over the `[ai]` block exactly the way
/// endpoint resolution layers it. `run_root` is the claimed run directory
/// (the scratch file's home, ADR 0003) and `scopes` the run's approved
/// capabilities: the scratch database is admitted here, once per run, when
/// — and only when — the run approved `scratch`, and the runner wiring is
/// built once per run the same way — the placement guard, the startup probe,
/// and the admission check — when the run approved a runner scope, fresh and
/// resume alike. `approval` is the run boundary's admitted mode (`mode.rs`):
/// the composition is unreachable in bypass mode, because no entry point can
/// construct the admitted type around it. `allow_tokens` are the run's
/// stated scopes as the grammar's words — the frozen decider's seeds and the
/// journal payload's carried words; a fresh run states them from `--allow`,
/// a resume from the journal. `profile_override` is the host's active
/// connection profile (what a nested child's `--profile` forwards); `None`
/// keeps the resolved default. Errors are configuration, connection, or
/// admission problems (exit-code class 3), reported as text for the caller
/// to emit.
pub(super) async fn assemble(
    runtime: &RuntimeConfig,
    profile_override: Option<&String>,
    run_root: &std::path::Path,
    scopes: &saya_types::Capabilities,
    workspace: std::sync::Arc<saya_harness::workspace::Workspace>,
    approval: RunApproval,
    allow_tokens: &[String],
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
            &runtime.secret_resolver(),
            &runtime.cache_scope,
            runtime.resolved.query_timeout_seconds,
            false,
            profile_name.as_deref().unwrap_or(""),
            primary,
            &[],
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
        .with_workspace(Some(workspace)),
    );
    // Shared per run, admitted before anything runs (fail closed at start,
    // never mid-flight): one scratch database when the run approved
    // `scratch`, its per-statement timeout the run's resolved query
    // timeout. The per-step toolsets put it behind the composites of the
    // steps that asked for it; a step that did not ask never sees it.
    let scratch =
        ScratchSql::admit(run_root, scopes)
            .map_err(|error| format!("scratch database could not be opened: {error}"))?
            .map(|scratch| {
                Arc::new(scratch.with_query_timeout(Duration::from_secs(
                    runtime.resolved.query_timeout_seconds,
                )))
            });
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
    // never mid-flight): the runner wiring — the sandbox composed over the
    // run's fs roots, the placement guard, the startup probe, and the
    // admission check — only when the run approved a runner scope. A run
    // that did not approve runner never consults the directory, and a host
    // the probe refused strips the scope from plan validation
    // (`plan_scopes`), so plans asking for it refuse as needs-approval.
    let wiring = build_runner(
        &runtime.resolved.jobs.runner,
        &runtime.resolved.jobs.interpreter,
        run_root,
        scopes,
    )?;
    // The run's decider is the frozen session policy (U4), seeded from the
    // run's stated scopes: a `sql:<connection>` token pre-answers
    // the read-shaped SQL calls that name the connection under `ask`
    // mode, and every other ask the seeds do not cover denies with the
    // engine's own reason. The composition facts are the run's own —
    // built from the approved scopes and the wiring, the same gates the
    // per-step toolsets build from — so a seed pre-answers exactly the
    // calls the composition carries (U8). The primary stays unbound —
    // only a call that names its connection suggests a token — and the
    // policy cannot accumulate: a headless session grant is impossible.
    let decider = TerminalApproval::frozen(
        approval.policy(),
        allow_tokens,
        super::decider_facts::for_frozen_decider(
            scopes,
            &wiring,
            fetch.as_ref(),
            &workspace_root,
            runtime,
        ),
    );
    Ok(Pieces {
        provider,
        tools,
        scratch,
        fetch,
        runner: wiring.runner,
        plan_scopes: wiring.plan_scopes,
        decider,
        model: ai.model,
        profile_names,
        allow_query_data,
    })
}
