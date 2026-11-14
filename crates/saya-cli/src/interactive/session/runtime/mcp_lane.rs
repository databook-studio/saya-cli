//! The interactive session's MCP servers, started once per session process.
//!
//! A server is started at acquisition and never again: every recomposition
//! carries the same lane forward, and every turn takes it under that turn's
//! own consent. The operator is deliberately not part of this composition —
//! a session's operator is the panel that is on screen when a call is made,
//! which is a fact of the turn, not of the session
//! ([`SessionUniverse::mcp_for_turn`]).
//!
//! [`SessionUniverse::mcp_for_turn`]:
//!     crate::interactive::session::universe::SessionUniverse::mcp_for_turn

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken};

use crate::agent::tools::consent::Caller;
use crate::mcp::client::McpTools;

/// Starts the configured servers, or `None` where the mode reaches none or
/// the configuration names none. A server that fails to start does not fail
/// the session: it is reported in the startup notice, and its tools are
/// simply absent.
pub(crate) fn compose(
    runtime: &crate::config::runtime::RuntimeConfig,
    mode: ApprovalPolicy,
) -> Option<Arc<McpTools>> {
    McpTools::connect(
        &runtime.resolved.mcp_servers,
        &runtime.secret_resolver(),
        mode,
        None,
        &Caller::Session,
        CancellationToken::new(),
    )
}
