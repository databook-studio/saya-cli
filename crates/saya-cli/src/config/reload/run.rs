//! The wired reload operation (O04): one shared step for every surface —
//! the headless REPL, `--turn-file`, and the TUI's queued-prompt drain. A
//! candidate that fails to build changes nothing and says so; a valid one
//! is planned against the running runtime, applied to the session state,
//! and swapped in by the caller (the swap is observed by the next turn).
//! Tests live in the sibling `run_tests.rs`.

use super::{ReloadInputs, apply_plan, build_candidate, plan_reload};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionState;

/// The outcome of one wired reload: the new runtime with the plan's render
/// text, or the pinned failure sentence with nothing mutated. `new` is boxed
/// (`RuntimeConfig` is a large resolved struct the failure arm must not pay
/// for); callers move it straight into their `Arc` handle.
pub(crate) enum ReloadOutcome {
    Reloaded { new: Box<RuntimeConfig>, text: String },
    Failed(String),
}

/// Builds a candidate from the SAME launch inputs, plans it against the
/// running runtime, re-seeds the session copies the plan names, and hands
/// back the new runtime plus the report text. A failed build mutates
/// nothing: the previous configuration stays in use, exactly as said.
pub(crate) fn reload_runtime(
    inputs: &ReloadInputs,
    old: &RuntimeConfig,
    state: &mut SessionState,
) -> ReloadOutcome {
    match build_candidate(inputs) {
        Err(error) => ReloadOutcome::Failed(format!(
            "Reload failed: {error}. The previous configuration is still in use."
        )),
        Ok(new) => {
            let plan = plan_reload(old, &new, state);
            apply_plan(&plan, state);
            ReloadOutcome::Reloaded {
                new: Box::new(new),
                text: plan.render(),
            }
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
