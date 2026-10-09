//! The MCP tool dispatch (tasks Da + Db): the single place a call is
//! admitted or refused before a body runs. Every tool body lives beside its
//! concern — [`super::schema_tool`] and [`super::query_tool`],
//! [`super::context_tools`] (contracts), [`super::replay_tools`]
//! (investigation_run) — and shares the helpers here: the startup allowlist
//! gate, the data-sharing gate, the sanitized error shape, and the response
//! bound.

use rmcp::{
    ErrorData, RoleServer,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode,
        RequestId, ServerResult,
    },
    service::{RequestContext, TxJsonRpcMessage},
};
use serde_json::json;

use super::{
    catalog, context::McpContext, context_tools, policy::ServePolicy, query_tool, replay_tools,
    schema_tool,
};

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tools_tests;

include!("tools/dispatch.rs");
include!("tools/wire_bounds.rs");
include!("tools/arguments.rs");
