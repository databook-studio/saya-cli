//! One connected MCP server: its process, the tools it advertises, and the
//! two requests the client makes of it after `initialize`.
//!
//! Everything a server says is untrusted. The listing pass applies every bound
//! in [`super::bounds`] to what it reads and **drops** what does not fit,
//! reporting each drop rather than rewriting it: a tool whose name is not a
//! name a provider's function field accepts, whose schema is not a bounded JSON
//! object, or whose name another tool of the same server already took, is a
//! tool this client will not offer. The alternative — renaming or truncating —
//! would put a contract in front of the model that the server never stated.
//!
//! A call's answer is bounded and redacted exactly as a `shell` call's output
//! is: the ring keeps both ends and the marker says what was dropped, and the
//! redaction pass runs over the retained text before anything downstream sees
//! it. The values this server was handed in its `env` are scrubbed as well —
//! they are the credentials it could echo back.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use saya_config::ResolvedMcpServer;
use saya_harness::runner::{OutputRing, output::capture};
use serde_json::{Map, Value, json};

use super::bounds::{
    MAX_LIST_PAGES, MAX_RESULT_BYTES, MAX_TOOL_DESCRIPTION_CHARS, MAX_TOOL_SCHEMA_BYTES,
    MAX_TOOLS_PER_SERVER,
};
use super::name;
use super::process::ServerProcess;
use super::protocol;

/// One tool a connected server offers, as this client advertises it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct McpTool {
    /// The server's own name for the tool, which is what a call sends back.
    pub(crate) tool: String,
    /// The name the model sees and calls: `mcp__<server>__<tool>`.
    pub(crate) exposed: String,
    /// The server's description, bounded, with a true prefix naming the
    /// server it came from.
    pub(crate) description: String,
    /// The server's input schema, verbatim and bounded.
    pub(crate) parameters: Value,
}

/// One live server.
pub(crate) struct McpServer {
    name: String,
    process: ServerProcess,
    tools: Vec<McpTool>,
    call_timeout: Duration,
    /// The values this server was handed in its environment, kept for the
    /// redaction pass over anything it echoes back.
    secrets: Vec<String>,
    /// Request ids. Unique within the server for the life of the connection.
    next_id: AtomicU64,
    /// Why each advertised tool this client dropped was dropped. Reported so
    /// a server that is silently unusable is never mistaken for a server with
    /// nothing to offer.
    drops: Vec<String>,
    /// Why this server stopped answering, once it has: the transport's reason
    /// plus the tail of the server's own stderr.
    failure: Mutex<Option<String>>,
}

impl McpServer {
    /// Starts `server` and completes the handshake, or refuses with the reason
    /// the server could not be used. A refusal is not fatal to the surface: the
    /// caller reports it and the server's tools are simply absent.
    pub(crate) fn connect(
        server: &ResolvedMcpServer,
        env: Vec<(String, String)>,
        secrets: Vec<String>,
    ) -> Result<Self, String> {
        let process = ServerProcess::spawn(server.command(), server.args(), &env)
            .map_err(|error| format!("mcp server {:?}: {error}", server.name()))?;
        let mut connected = Self {
            name: server.name().to_owned(),
            process,
            tools: Vec::new(),
            call_timeout: server.call_timeout(),
            secrets,
            next_id: AtomicU64::new(1),
            drops: Vec::new(),
            failure: Mutex::new(None),
        };
        connected
            .handshake(server.startup_timeout())
            .map_err(|error| format!("mcp server {:?}: {error}", server.name()))?;
        connected.tools = connected
            .list_tools(server.startup_timeout())
            .map_err(|error| format!("mcp server {:?}: {error}", server.name()))?;
        Ok(connected)
    }

    /// The server's configured name.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The tools this server contributes, in the order it advertised them.
    pub(crate) fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    /// Why each advertised tool this client dropped was dropped, with the
    /// reason for each.
    pub(crate) fn drops(&self) -> &[String] {
        &self.drops
    }

    /// The `initialize` exchange and the `notifications/initialized` that
    /// follows it.
    fn handshake(&mut self, timeout: Duration) -> Result<(), String> {
        let id = self.next_id();
        let frame = protocol::request(id, "initialize", protocol::initialize_params());
        let reply = self.process.exchange(&frame, id, timeout)?;
        let reply: Value = serde_json::from_slice(&reply)
            .map_err(|_| "the initialize reply was not JSON".to_owned())?;
        protocol::read_initialize(&reply)?;
        // The notification carries no id, so there is nothing to wait for: the
        // server may not answer it at all, which is correct.
        let frame = protocol::notification("notifications/initialized", json!({}));
        self.process
            .write(&frame)
            .map_err(|error| format!("could not complete the handshake: {error}"))
    }

    /// Every page of `tools/list`, bounded by page count and tool count.
    fn list_tools(&mut self, timeout: Duration) -> Result<Vec<McpTool>, String> {
        let mut tools: Vec<McpTool> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let id = self.next_id();
            let frame =
                protocol::request(id, "tools/list", protocol::list_params(cursor.as_deref()));
            let reply = self.process.exchange(&frame, id, timeout)?;
            let reply: Value = serde_json::from_slice(&reply)
                .map_err(|_| "a tools/list reply was not JSON".to_owned())?;
            let page = protocol::read_tool_page(&reply)?;
            for tool in page.tools {
                if tools.len() >= MAX_TOOLS_PER_SERVER {
                    self.note_drop(format!(
                        "more than {MAX_TOOLS_PER_SERVER} tools were advertised; the rest were \
                         not offered"
                    ));
                    break;
                }
                if let Some(offered) = self.offer(&tool) {
                    tools.push(offered);
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(tools),
            }
        }
        self.note_drop(format!(
            "the server kept listing more than {MAX_LIST_PAGES} pages of tools; the rest were \
             not offered"
        ));
        Ok(tools)
    }

    /// One advertised tool, bounded, or `None` where it is dropped.
    fn offer(&mut self, tool: &protocol::RawTool) -> Option<McpTool> {
        if !name::is_exposable(&tool.name) {
            self.note_drop(format!(
                "tool {:?} was not offered: its name is not one a tool call can carry",
                tool.name
            ));
            return None;
        }
        let exposed = name::exposed(&self.name, &tool.name);
        if self
            .tools
            .iter()
            .any(|existing| existing.exposed == exposed)
        {
            self.note_drop(format!(
                "tool {:?} was not offered: the name {exposed} was already taken",
                tool.name
            ));
            return None;
        }
        if !tool.input_schema.is_object() {
            self.note_drop(format!(
                "tool {:?} was not offered: its input schema is not a JSON object",
                tool.name
            ));
            return None;
        }
        let schema_bytes = serde_json::to_vec(&tool.input_schema).map_or(usize::MAX, |b| b.len());
        if schema_bytes > MAX_TOOL_SCHEMA_BYTES {
            self.note_drop(format!(
                "tool {:?} was not offered: its input schema is {schema_bytes} bytes, over the \
                 {MAX_TOOL_SCHEMA_BYTES}-byte limit",
                tool.name
            ));
            return None;
        }
        Some(McpTool {
            tool: tool.name.clone(),
            exposed,
            description: bounded_description(&self.name, &tool.description),
            parameters: tool.input_schema.clone(),
        })
    }

    /// Records one drop reason.
    fn note_drop(&mut self, reason: String) {
        self.drops.push(reason);
    }

    /// One `tools/call`, reduced to the text the model reads.
    pub(crate) fn call(&self, tool: &str, arguments: &Value) -> Result<Value, String> {
        let id = self.next_id();
        let frame = protocol::request(id, "tools/call", protocol::call_params(tool, arguments));
        let reply = self
            .process
            .exchange(&frame, id, self.call_timeout)
            .inspect_err(|reason| self.note_failure(reason.clone()))?;
        let reply: Value = serde_json::from_slice(&reply)
            .map_err(|_| "the tools/call reply was not JSON".to_owned())?;
        let outcome = protocol::read_call(&reply)?;
        let structured = protocol::read_structured(&reply);
        let ring = OutputRing::new(MAX_RESULT_BYTES);
        ring.push(outcome.text.as_bytes());
        let captured = capture(&ring, &self.secrets);
        let mut result = Map::new();
        result.insert("server".to_owned(), Value::String(self.name.clone()));
        result.insert("tool".to_owned(), Value::String(tool.to_owned()));
        result.insert("content".to_owned(), Value::String(captured.text));
        if outcome.is_error {
            result.insert("is_error".to_owned(), Value::Bool(true));
        }
        if captured.truncated {
            result.insert("truncated".to_owned(), Value::Bool(true));
            result.insert(
                "dropped_bytes".to_owned(),
                Value::Number(captured.dropped_bytes.into()),
            );
        }
        if let Some(structured) = structured {
            result.insert("structured_content".to_owned(), structured);
        }
        Ok(Value::Object(result))
    }

    /// Why this server stopped answering, once it has: the reason the
    /// transport recorded, plus the tail of its own stderr.
    fn note_failure(&self, reason: String) {
        let tail = self.process.stderr_tail();
        let full = if tail.is_empty() {
            reason
        } else {
            format!("{reason}; its last output was: {tail}")
        };
        let mut failure = self
            .failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failure.is_none() {
            *failure = Some(full);
        }
    }

    /// The recorded failure, if the server has one.
    pub(crate) fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The next request id.
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::AcqRel)
    }
}

/// A tool's description as the model reads it: a true prefix naming the server
/// it came from, then the server's own words, bounded. The prefix is what
/// makes the tool's origin legible at the point of use — a third-party tool's
/// description is its own claim about itself, and the model is owed the fact
/// that it did not come from saya.
fn bounded_description(server: &str, description: &str) -> String {
    let prefix = format!("[MCP server {server:?}] ");
    // One character is kept back for the marker, so a truncated description is
    // exactly the cap and never one over it.
    let room = MAX_TOOL_DESCRIPTION_CHARS
        .saturating_sub(prefix.chars().count())
        .max(1);
    let mut text: String = description.chars().take(room).collect();
    if description.chars().count() > room {
        text.pop();
        text.push('…');
    }
    format!("{prefix}{text}")
}
