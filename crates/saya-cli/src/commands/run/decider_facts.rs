//! The frozen decider's composition facts: what the run's own composition
//! carries, read off the same pieces the per-step toolsets build from — the
//! approved scopes, the runner wiring the probe proved, the admitted
//! scratch and fetch members, and the workspace root. A `--allow` seed
//! pre-answers exactly the calls this composition can carry, never a
//! token that gates nothing (U8: the suggester consults the composition on
//! every surface, and the headless run's decider is no exception — its
//! seeds were admitted against these same gates at start, `runner.rs`'s
//! admission checks, so an honest seed always matches).

use crate::approval_facts::{ApprovalFacts, FetchFacts, RunnerFacts, ScratchFacts};
use crate::config::runtime::RuntimeConfig;
use saya_types::Capabilities;
use std::path::Path;

use super::runner::RunnerWiring;
use super::tools::RunFetch;

/// The composition facts for the run's frozen decider. The frozen decider
/// renders no prompts — `can_prompt` is false — so these facts are the
/// carried/un-carried gate per grant family, filled with the enforcement's
/// own numbers where a member exists.
pub(super) fn for_frozen_decider(
    scopes: &Capabilities,
    wiring: &RunnerWiring,
    fetch: Option<&RunFetch>,
    workspace_root: &Path,
    runtime: &RuntimeConfig,
) -> ApprovalFacts {
    // The fetch member's lane bounds: the same constructor the session's
    // facts and the run's toolsets use, the same numbers by construction.
    let lane = saya_harness::fetch::FetchLimits::for_tool_lane();
    ApprovalFacts {
        runner: wiring.runner.as_ref().map(|runner| RunnerFacts {
            fs_roots: runner.spawn.fs_roots().to_vec(),
            net_allow: runner.spawn.net_allow().to_vec(),
            timeout_seconds: runner.timeout.as_secs(),
            // The approved programs — the superset of every step's narrowed
            // scope, the universe the admission check ran against.
            runner_programs: scopes
                .runner
                .as_ref()
                .map(|scope| scope.programs.clone())
                .unwrap_or_default(),
            interpreter_programs: scopes
                .interpreter
                .as_ref()
                .map(|scope| scope.programs.clone())
                .unwrap_or_default(),
            // The run's credential seam resolves nothing (`tools.rs` builds
            // the empty resolver); no declaration surface exists yet.
            credentials_declared: 0,
        }),
        // The admitted members, where the scopes approved them — the same
        // `ScratchSql::admit` / fetch-scope gates `assemble` already ran.
        scratch: scopes.scratch.then_some(ScratchFacts {
            row_cap: saya_harness::scratch::SCRATCH_ROW_CAP,
            timeout_seconds: runtime.resolved.query_timeout_seconds,
        }),
        fetch: fetch.map(|fetch| FetchFacts {
            fetch_body_bytes: lane.max_total_bytes,
            fetch_seconds: lane.time_budget.as_secs(),
            fetch_redirects: lane.max_redirect_hops,
            download: Some(fetch.budget.clone()),
        }),
        // The write-shaped family exists only where the run approved it —
        // the per-step toolsets gate `workspace_write` on the step's
        // capability, which plan validation keeps inside the run's scopes.
        workspace_root: scopes.workspace_write.then(|| workspace_root.to_path_buf()),
        // Runs never get the host lane: the run surface refuses command:
        // scopes at parse (permanent policy refusal), so the frozen decider
        // carries no host member to consult.
        host: None,
        ..ApprovalFacts::default()
    }
}
