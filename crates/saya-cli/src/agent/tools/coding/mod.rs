//! Native parent tools adapt the same job owner used by terminal controls.
pub(crate) mod child;
mod parent;
pub(crate) use parent::{CodingTools, definitions};
use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::Value;
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
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}
