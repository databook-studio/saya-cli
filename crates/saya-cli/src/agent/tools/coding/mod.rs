//! Native parent tools adapt the same job owner used by terminal controls.
mod direct;
mod direct_tools;
mod owner_denial;
mod parent;
pub(crate) mod plan_gate;
mod readonly;
mod root_mcp;
pub(crate) use root_mcp::build_for_helper;
mod root_progress;
mod root_rewind;
mod root_shell;
mod root_web;
mod routes;
/// The host lane's secret stripping, shared with every other surface that
/// composes a `shell` (a run's per-step toolsets strip the same names).
pub(crate) mod shell_secrets;
pub(crate) mod task_tool;
pub(crate) use direct::RootTools;
pub(crate) use parent::definitions;
#[cfg(test)]
pub(crate) use root_rewind::RootRewind;

#[cfg(test)]
#[path = "direct_allow_tests.rs"]
mod direct_allow_tests;

#[cfg(test)]
#[path = "direct_ask_tests.rs"]
mod direct_ask_tests;

#[cfg(test)]
#[path = "direct_tests.rs"]
mod direct_tests;

#[cfg(test)]
#[path = "direct_web_tests.rs"]
mod direct_web_tests;

#[cfg(test)]
#[path = "helpers_concurrency_tests.rs"]
mod helpers_concurrency_tests;

#[cfg(test)]
#[path = "readonly_tests.rs"]
mod readonly_tests;

#[cfg(test)]
#[path = "root_rewind_advertisement_tests.rs"]
mod root_rewind_advertisement_tests;

#[cfg(test)]
#[path = "root_shell_advertisement_tests.rs"]
mod root_shell_advertisement_tests;

#[cfg(test)]
#[path = "root_shell_background_tests.rs"]
mod root_shell_background_tests;

#[cfg(test)]
#[path = "root_shell_tests.rs"]
mod root_shell_tests;

#[cfg(test)]
#[path = "root_shell_output_bound_tests.rs"]
mod root_shell_output_bound_tests;

#[cfg(test)]
#[path = "root_shell_exit_tests.rs"]
mod root_shell_exit_tests;

#[cfg(test)]
#[path = "root_shell_silent_read_tests.rs"]
mod root_shell_silent_read_tests;

#[cfg(test)]
#[path = "plan_gate_tests.rs"]
mod plan_gate_tests;

#[cfg(test)]
#[path = "notebook_advertisement_tests.rs"]
mod notebook_advertisement_tests;

#[cfg(test)]
#[path = "tool_order_tests.rs"]
mod tool_order_tests;

#[cfg(test)]
#[path = "tests_lean_advertisement.rs"]
mod tests_lean_advertisement;
use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::Value;

/// Child delegation stays unused unless the operator asked for a helper.
pub(crate) const OPTIONAL_DELEGATION: &str = "Optional. Use this only when the user asked you to \
hand work to a helper; otherwise do the work directly. ";

/// The spec-fidelity rule every delegation description carries: the operator's
/// request outranks anything the coordinator adds to an assignment. The root
/// prompt states the same rule in the same words; one constant keeps the three
/// surfaces from drifting apart.
pub(crate) const REQUEST_AUTHORITY_RULE: &str = "The user's request is the only authority for \
requirements: do not invent requirements, formats or conventions where it leaves room, prefer \
the most conventional reading, and mark any interpretation you add as 'my reading' with the \
request sentence it interprets.";

/// The shared shape of the coding tools: the write flag decides approval and
/// local state. They are `Concurrent`, so the agent runner may overlap several
/// calls from one turn — `helpers` is what that buys, several helpers at once.
pub(crate) fn definition(
    name: &str,
    description: &str,
    parameters: Value,
    write: bool,
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        read_only: !write,
        parameters,
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: write,
            local_state: if write {
                LocalStateEffect::WriteWorkspace
            } else {
                LocalStateEffect::Read
            },
        },
        concurrency: ToolConcurrency::Concurrent,
        completion: None,
    }
}
