//! The direct-mode root's own workspace calls, and what they may have changed.
//!
//! The root's toolset reports every call that could have touched the working
//! checkout W. A direct root edits W directly, so any ATTEMPT that may write —
//! whatever its outcome, and whether or not it succeeded — invalidates the
//! advisory readiness cache: a refused or failed call can still have written,
//! and the advisory view must not keep claiming facts about a tree that may
//! have moved.

use crate::commands::coding::NativeCodingJob;

/// Whether a root workspace call may have changed W: the three anchored write
/// tools, plus the shell, whose side effects on W the call's own result cannot
/// rule out.
pub(super) fn may_change_workspace(name: &str) -> bool {
    crate::agent::tools::workspace::is_write_tool(name)
        || crate::agent::tools::workspace::canonical_name(name) == "shell"
}

/// Notes one root workspace call. A call that may have changed W invalidates
/// the advisory readiness cache; a read changes nothing and is ignored.
pub(super) fn note(job: &NativeCodingJob, name: &str) {
    if !may_change_workspace(name) {
        return;
    }
    job.note_root_w_attempt();
}

#[cfg(test)]
mod tests {
    use super::may_change_workspace;

    #[test]
    fn only_the_write_tools_and_the_shell_may_change_the_workspace() {
        for name in ["write", "edit", "delete", "shell", "bash"] {
            assert!(may_change_workspace(name), "{name} may change W");
        }
        for name in ["read", "list", "grep", "glob", "helpers"] {
            assert!(!may_change_workspace(name), "{name} changes nothing");
        }
    }
}
