//! The MCP **client** side: the tools of the servers a user configured.
//!
//! `crate::mcp` is the server side — `saya mcp serve` speaks this protocol to
//! other clients. This module is the other direction, and it is the only place
//! in the tree where a *third-party* tool definition reaches the model. That
//! makes it a trust boundary, and every part of the design follows from it:
//!
//! - **Off until configured.** With no `[mcp_servers]` table no server exists,
//!   no process starts, and no surface advertises an `mcp__` tool.
//! - **A protected table.** The table is protected whole (see
//!   `saya_config::layers`), so a repository's `.saya/config.toml` can neither
//!   add a server, replace one, nor retarget one at a different program.
//! - **A namespaced, bounded advertisement.** A server's tools reach the model
//!   as `mcp__<server>__<tool>`. Names, descriptions, schemas, tool counts, and
//!   the frame a server may send are all capped ([`bounds`]), and a tool that
//!   does not fit is **dropped** rather than rewritten — a renamed or truncated
//!   tool would put a contract in front of the model that the server never
//!   stated.
//! - **Consent per call.** A server is a program whose effects saya cannot
//!   bound, so every call follows the `shell`'s own rule: `bypass` runs it,
//!   `ask` puts it to the operator, and a headless `ask` refuses with the flag
//!   that would reach it named.
//! - **A bounded, redacted result.** What comes back is cut to a byte cap with
//!   both ends kept, and the credential values the server was handed are
//!   scrubbed out before the model reads a byte.
//! - **A lifecycle that ends.** Servers start once, lazily, for the session or
//!   job that composes them; each one is killed and reaped when its client drops,
//!   and one that stops answering is killed rather than waited on.
//!
//! [`tools::McpTools`] is the value a surface holds: one advertisement, one
//! dispatch, and one approval decision, so a surface cannot offer a tool it
//! could not serve.

pub(crate) mod bounds;
mod name;
mod process;
mod protocol;
mod server;
mod tools;

pub(crate) use tools::{Consent, McpTools, advertised};

#[cfg(test)]
pub(crate) mod fixture;

/// The fixture harness: a configured server built through the real resolve
/// path, and the fakes a consent test needs. `pub(crate)` rather than private
/// to `tests`, because the catalog drift test in `agent::tools` composes a
/// real MCP client — a name this client advertises must be governed by the same
/// `[tools] allow` vocabulary every other advertised name is.
#[cfg(test)]
pub(crate) mod support;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
