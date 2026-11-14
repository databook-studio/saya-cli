use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};

pub(in crate::agent::tools) fn append_definitions(
    tools: &mut Vec<ToolDefinition>,
    permit_workspace_writes: bool,
) {
    append_workspace_read_tools(tools);
    append_workspace_write_tools(tools, permit_workspace_writes);
}

include!("definitions/reads.rs");
include!("definitions/writes.rs");
