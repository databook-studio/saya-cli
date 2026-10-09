//! The step's tool definitions: the run's tool universe narrowed to the
//! step's capabilities — hidden, not advertised-and-refused.

use saya_agent::{LocalStateEffect, ToolDefinition};

use saya_types::Capabilities;

/// Narrows the run's tool universe to the step's capabilities. A tool whose
/// local-state effect needs a capability the step lacks is **absent** from
/// the result — the hidden-not-advertised pattern: the model never sees a
/// tool it would only be refused on. Candidate-write tools are not
/// capability-scoped, so they pass and are stopped by the pinned limit.
///
/// The write-shaped class is one `LocalStateEffect` variant shared by
/// workspace-write, scratch, runner, interpreter and fetch
/// (`http_download`), so the filter asks the same question the permit
/// mapping does — "did this step approve some write-shaped scope?" — not
/// which tool it is: which tool is the composition root's decision, made
/// from the same capabilities the toolset was built from. This is the second
/// lock behind construction; it must never strip a tool the step approved.
pub(crate) fn definitions(
    universe: &[ToolDefinition],
    capabilities: &Capabilities,
) -> Vec<ToolDefinition> {
    universe
        .iter()
        .filter(|tool| match tool.effect.local_state {
            LocalStateEffect::WriteWorkspace => {
                capabilities.workspace_write
                    || capabilities.scratch
                    || capabilities.runner.is_some()
                    || capabilities.interpreter.is_some()
                    || capabilities.fetch.is_some()
            }
            // A session-metadata tool is always present: a run step's universe
            // is composed from declared capabilities, and no capability names
            // the session record — nor could one, since a step that could not
            // record what it is doing would be absurd. The arm is written out
            // so the decision stays deliberate instead of falling to the
            // catch-all by accident.
            LocalStateEffect::WriteSession => true,
            _ => true,
        })
        .cloned()
        .collect()
}
