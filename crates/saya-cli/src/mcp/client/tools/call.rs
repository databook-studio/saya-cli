//! One approved call to a tool of a connected server.
//!
//! Three gates stand in front of the server, in this order, and each one is a
//! statement about the call rather than about the server's answer: the mode's
//! refusal ([`McpTools::refusal`]), the name the client actually advertises, and
//! the size of the arguments. Only then is the request written, and only then
//! is the answer bounded and redacted ([`McpServer::call`]).
//!
//! The call itself runs on a blocking worker — the transport is a pipe with a
//! deadline, not a future — and the turn's cancellation races it, so a turn
//! that ends does not leave the runtime holding a third-party program's reply.

use std::sync::Arc;

use saya_agent::ToolError;
use serde_json::Value;

use super::McpTools;
use crate::mcp::client::bounds::MAX_ARGUMENT_BYTES;

impl McpTools {
    /// One approved call: the server's own answer, bounded and redacted.
    pub(crate) async fn call(&self, name: &str, arguments: &Value) -> Result<Value, ToolError> {
        if let Some(reason) = self.refusal() {
            return Err(ToolError::Runner(reason.to_owned()));
        }
        let Some((server, entry)) = self.entry(name) else {
            return Err(ToolError::UnsupportedTool);
        };
        let encoded = serde_json::to_vec(arguments).map_err(|_| ToolError::ArgumentsNotObject)?;
        if encoded.len() > MAX_ARGUMENT_BYTES {
            return Err(ToolError::Runner(format!(
                "the arguments for {name} are {} bytes; the limit is {MAX_ARGUMENT_BYTES}",
                encoded.len()
            )));
        }
        if let Some(reason) = server.failure() {
            return Err(ToolError::Runner(reason));
        }
        let server = Arc::clone(server);
        let tool = entry.tool.clone();
        let arguments = arguments.clone();
        let call = tokio::task::spawn_blocking(move || server.call(&tool, &arguments));
        let outcome = tokio::select! {
            biased;
            () = self.cancellation.cancelled() => {
                return Err(ToolError::Runner("the MCP tool call was cancelled".to_owned()));
            }
            outcome = call => outcome,
        };
        match outcome {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(reason)) => Err(ToolError::Runner(reason)),
            Err(_) => Err(ToolError::Runner(
                "the MCP tool call could not be completed".to_owned(),
            )),
        }
    }
}
