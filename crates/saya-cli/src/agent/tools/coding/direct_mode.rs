//! Which of the root's tools the run's mode reaches: the advertisement's mode
//! gate, read from the old session's own family rules.
//!
//! The session that ran before this one decided each family once — the
//! workspace file family (`write`, `edit`), the two fetch doors, the
//! host-command lane, the job shell — and answered "advertise it exactly where
//! its call can actually run": an approval surface that can answer the ask, the
//! mode's own consent, or a held grant for that family, which pre-answers the
//! call the mode would have asked about. This module reads those decisions
//! rather than restating them, so a mode's surface cannot drift from the
//! consent it resolves.
//!
//! Availability is not authority. A tool this gate withholds is not merely
//! unadvertised: the executor refuses the call at the door with the unknown-tool
//! error, which is what a call to a tool the surface never offered has always
//! received. A tool it admits is still decided per call by the same deciders.

use saya_agent::{AgentMode, ToolDefinition, read_only_permits};

use super::direct_tools::DirectCodingTools;
use super::root_rewind::REWIND;
use crate::interactive::session::permissions::{grant_families, grant_surface};

impl DirectCodingTools {
    /// Whether the run's mode reaches `definition`. A read-shaped tool is
    /// always offered — the SQL tools, the search tools, the session's own
    /// `checklist`. A write-shaped one is offered where its family's rule says
    /// a call to it can run:
    ///
    /// - the workspace file family — `write`, `edit`, the notebook writer, and
    ///   this root's `rewind`, which writes the checkout — through
    ///   [`grant_surface::workspace_files_available`];
    /// - the two fetch doors through [`grant_families::fetch_available`];
    /// - the host-command lane through [`grant_families::command_available`].
    ///
    /// The shell family is the mode's own: the root's shell is composed only
    /// where the mode keeps a lane for it, a sandbox proves, and — where the
    /// lane is the host's — somebody can answer its call. Its presence already
    /// is the mode's answer, so nothing here re-decides it. A write-shaped tool
    /// the old rule never named — the delegation, whose helpers carry their own
    /// approvals — keeps its own composition too.
    pub(super) fn mode_permits(&self, definition: &ToolDefinition) -> bool {
        if Self::is_shell(&definition.name) || read_only_permits(&definition.effect) {
            return true;
        }
        let job = &self.coordinator.job;
        let mode = job.approval_mode().get();
        let can_obtain_approval = job.can_obtain_approval();
        // The session's own seed store: what the launch stated, held beside the
        // mode. A token it holds pre-answers the call the mode would ask about,
        // so its family is advertised — the old session's own grant arm.
        let grants = job.launch_grants();
        // A working posture, not a plan one: a closed plan gate is the plan
        // gate's own advertisement decision, applied beside this one.
        let posture = AgentMode::Build;
        match definition.name.as_str() {
            "http_fetch" | "http_download" => {
                grant_families::fetch_available(posture, mode, can_obtain_approval, Some(&grants))
            }
            "run_command" => {
                grant_families::command_available(posture, mode, can_obtain_approval, Some(&grants))
            }
            "write" | "edit" | "notebook_edit" | REWIND => {
                grant_surface::workspace_files_available(
                    posture,
                    mode,
                    can_obtain_approval,
                    Some(&grants),
                )
            }
            _ => true,
        }
    }
}
