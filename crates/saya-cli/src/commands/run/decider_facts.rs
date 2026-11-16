//! The frozen decider's composition facts: what the run's own composition
//! carries, read off the same pieces the per-step toolsets build from — the
//! approved scopes, the admitted fetch member, and the
//! workspace root. A `--allow` seed pre-answers exactly the calls this
//! composition can carry, never a token that gates nothing. The suggester
//! consults the composition on every surface, and the headless run's decider
//! is no exception — its seeds are admitted against these same gates at
//! start, so an honest seed always matches.
//!
//! The runner and interpreter families are not on this list: a run's steps
//! run commands through the `shell` tool, whose own lane decision answers a
//! `shell` call before these facts are consulted at all, so no runner-shaped
//! fact belongs here.

use crate::approval_facts::{ApprovalFacts, FetchFacts};
use saya_types::Capabilities;
use std::path::Path;

use super::tools::RunFetch;

/// The composition facts for the run's frozen decider. The frozen decider
/// renders no prompts — `can_prompt` is false — so these facts are the
/// carried/un-carried gate per grant family, filled with the enforcement's
/// own numbers where a member exists.
pub(super) fn for_frozen_decider(
    scopes: &Capabilities,
    fetch: Option<&RunFetch>,
    workspace_root: &Path,
) -> ApprovalFacts {
    // The fetch member's lane bounds: the same constructor the session's
    // facts and the run's toolsets use, the same numbers by construction.
    let lane = saya_harness::fetch::FetchLimits::for_tool_lane();
    ApprovalFacts {
        // The admitted member, where the scope approved it — the same
        // fetch-scope gate `assemble` already ran.
        fetch: fetch.map(|fetch| FetchFacts {
            fetch_body_bytes: lane.max_total_bytes,
            fetch_seconds: lane.time_budget.as_secs(),
            fetch_redirects: lane.max_redirect_hops,
            download: Some(fetch.budget.clone()),
        }),
        // The write-shaped family exists only where the run approved it —
        // the per-step toolsets gate `write` on the step's
        // capability, which plan validation keeps inside the run's scopes.
        workspace_root: scopes.workspace_write.then(|| workspace_root.to_path_buf()),
        // Runs never get the host lane: the run surface refuses command:
        // scopes at parse (permanent policy refusal), so the frozen decider
        // carries no host member to consult.
        host: None,
        ..ApprovalFacts::default()
    }
}
