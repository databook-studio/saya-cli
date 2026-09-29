//! The stdio MCP server: rmcp glue for `saya mcp serve` (task Da, ADR 0008).
//!
//! A hand-written [`ServerHandler`] — no macros feature. The tool catalog and
//! bodies live in [`super::tools`]; this file is the bounded plumbing around
//! them: in-flight cap, request and response bounds, call timeout. Nothing
//! but JSON-RPC ever reaches stdout — the transport writes protocol frames
//! only, and every diagnostic goes to stderr.

use std::sync::atomic::{AtomicUsize, Ordering};

use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode,
        Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
    },
    service::{QuitReason, RequestContext},
    transport::stdio,
};

use super::{
    policy::{MAX_REQUEST_BYTES, ServePolicy},
    tools,
};

pub(crate) struct SayaServer {
    policy: ServePolicy,
    in_flight: AtomicUsize,
}

impl ServerHandler for SayaServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("saya", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Bounded, read-only database access for the profiles this server \
                 was started with. `list_profiles` lists them: names and dialects \
                 only, never paths, hosts, or identities.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::advertised()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.run_tool(request).await
    }
}

impl SayaServer {
    pub(crate) fn new(policy: ServePolicy) -> Self {
        Self {
            policy,
            in_flight: AtomicUsize::new(0),
        }
    }

    /// One bounded tool call: acquire an in-flight slot, refuse an oversized
    /// request, run the body under the call timeout. Every hook is the one
    /// the data tools (task Db) inherit.
    async fn run_tool(
        &self,
        request: CallToolRequestParams,
    ) -> Result<CallToolResponse, ErrorData> {
        let _slot = self.acquire_in_flight()?;
        if let Some(arguments) = request.arguments.as_ref()
            && !arguments.is_empty()
        {
            let bytes = serde_json::to_vec(arguments).map_err(unserializable_arguments)?;
            if !self.policy.request_allowed(bytes.len()) {
                return Err(ErrorData::invalid_params(
                    format!(
                        "tool arguments exceed the {}-byte request bound",
                        MAX_REQUEST_BYTES
                    ),
                    None,
                ));
            }
        }
        match tokio::time::timeout(
            self.policy.call_timeout(),
            tools::run(&self.policy, &request),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "the tool call exceeded the {}-second bound",
                self.policy.call_timeout().as_secs()
            ))])
            .into()),
        }
    }

    pub(crate) fn acquire_in_flight(&self) -> Result<InFlightSlot<'_>, ErrorData> {
        if self.in_flight.fetch_add(1, Ordering::Relaxed) >= self.policy.max_in_flight() {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "too many in-flight tool calls; retry once one completes",
                None,
            ));
        }
        Ok(InFlightSlot {
            counter: &self.in_flight,
        })
    }
}

/// Releases the in-flight slot on every exit path, including an error return
/// or a dropped future.
pub(crate) struct InFlightSlot<'a> {
    counter: &'a AtomicUsize,
}

impl Drop for InFlightSlot<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

fn unserializable_arguments(_: serde_json::Error) -> ErrorData {
    ErrorData::invalid_params("tool arguments are not serializable", None)
}

/// Serve stdio until stdin closes, then report the process exit code. A
/// client that hangs up before completing the handshake is a failed
/// connection (exit 2); a completed session ends cleanly on EOF (exit 0),
/// as does an explicit cancel.
pub(crate) async fn run(policy: ServePolicy) -> Result<i32, Box<dyn std::error::Error>> {
    let running = SayaServer::new(policy).serve(stdio()).await?;
    match running.waiting().await? {
        QuitReason::Closed | QuitReason::Cancelled => Ok(0),
        other => Err(format!("mcp server task failed: {other:?}").into()),
    }
}
