//! The three JSON-RPC requests this client makes, and the reading of their
//! answers.
//!
//! The wire is newline-delimited JSON-RPC 2.0 — the same framing `saya mcp
//! serve` writes — so this module is only the *shapes*: what a request looks
//! like, and what a reply has to carry before anything downstream may believe
//! it. Everything here is a pure function of bytes, which is what makes the
//! whole protocol testable without a process.
//!
//! A server's answer is untrusted input. Every reader below is total: a reply
//! that is not an object, has no `result`, or carries a field of the wrong
//! type is a typed refusal naming the request that failed, never a panic and
//! never a silently empty list.

use serde_json::{Value, json};

use super::bounds::PROTOCOL_VERSION;

/// One outbound request: a JSON-RPC call with a numeric id. The line is what
/// the transport writes, `\n` included, so the transport never has to know
/// the framing.
pub(crate) fn request(id: u64, method: &str, params: Value) -> Vec<u8> {
    let mut frame = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .unwrap_or_else(|_| unreachable!("a request built from owned values always serialises"));
    frame.push(b'\n');
    frame
}

/// One outbound notification: a JSON-RPC message with no id and no reply.
pub(crate) fn notification(method: &str, params: Value) -> Vec<u8> {
    let mut frame = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    }))
    .unwrap_or_else(|_| unreachable!("a notification built from owned values always serialises"));
    frame.push(b'\n');
    frame
}

/// The `initialize` parameters. The client states no capabilities: it makes
/// three plain requests and needs no sampling, roots, or elicitation from the
/// server, so it claims none.
pub(crate) fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "saya", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// The `tools/list` parameters, with the cursor a previous page returned.
pub(crate) fn list_params(cursor: Option<&str>) -> Value {
    match cursor {
        Some(cursor) => json!({ "cursor": cursor }),
        None => json!({}),
    }
}

/// The `tools/call` parameters.
pub(crate) fn call_params(tool: &str, arguments: &Value) -> Value {
    json!({ "name": tool, "arguments": arguments })
}

/// One tool as the server described it, before any bound is applied.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
}

/// One page of `tools/list`: the tools it carried, and the cursor for the next
/// page when the server said there is one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolPage {
    pub(crate) tools: Vec<RawTool>,
    pub(crate) next_cursor: Option<String>,
}

/// Reads a reply's `result`, or the server's own error message.
///
/// A JSON-RPC error is a normal answer from a working server — a bad cursor, a
/// tool that does not exist — so it is a refusal carrying the server's own
/// words rather than a transport failure.
pub(crate) fn result_of<'a>(reply: &'a Value, method: &str) -> Result<&'a Value, String> {
    let Some(object) = reply.as_object() else {
        return Err(format!("the server's {method} reply was not a JSON object"));
    };
    if let Some(error) = object.get("error") {
        return Err(format!(
            "the server refused {method}: {}",
            error_message(error)
        ));
    }
    object
        .get("result")
        .ok_or_else(|| format!("the server's {method} reply carried no result"))
}

/// The text of a JSON-RPC `error` object, bounded and never empty.
fn error_message(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("no message");
    let code = error.get("code").and_then(Value::as_i64);
    let mut text = match code {
        Some(code) => format!("{message} (code {code})"),
        None => message.to_owned(),
    };
    text.truncate(512);
    text
}

/// Whether a reply belongs to the request with `id`. A server's own
/// notifications carry no id and are skipped by the caller.
pub(crate) fn reply_id(reply: &Value) -> Option<u64> {
    reply.get("id").and_then(Value::as_u64)
}

/// Reads an `initialize` answer. The server's stated protocol version is
/// accepted as-is; what is checked is that it answered at all, which is the
/// only thing the three requests this client makes depend on.
pub(crate) fn read_initialize(reply: &Value) -> Result<String, String> {
    let result = result_of(reply, "initialize")?;
    let version = result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| "the server's initialize reply carried no protocolVersion".to_owned())?;
    Ok(version.to_owned())
}

/// Reads one `tools/list` page. A tool entry that is not an object, or carries
/// no string `name`, is skipped rather than failing the page: one malformed
/// entry must not cost the server every other tool it advertises.
pub(crate) fn read_tool_page(reply: &Value) -> Result<ToolPage, String> {
    let result = result_of(reply, "tools/list")?;
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| "the server's tools/list reply carried no tools array".to_owned())?;
    let mut page = Vec::with_capacity(tools.len());
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        page.push(RawTool {
            name: name.to_owned(),
            description: tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            // An absent schema is an empty object, not a refusal: a tool that
            // takes no arguments is a tool, and `{}` is what it takes.
            input_schema: tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object" })),
        });
    }
    let next_cursor = result
        .get("nextCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    Ok(ToolPage {
        tools: page,
        next_cursor,
    })
}

/// One `tools/call` answer, already reduced to what the model reads: the
/// server's own text, and whether the server reported the call as failed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CallOutcome {
    pub(crate) text: String,
    pub(crate) is_error: bool,
}

/// Reads a `tools/call` answer.
///
/// The content array is flattened to text: `text` items are joined verbatim,
/// and any other item kind is named by its type rather than dropped silently,
/// so the model is never shown an answer that claims to be empty when it was
/// not. The `isError` flag is the server's own statement about the call.
pub(crate) fn read_call(reply: &Value) -> Result<CallOutcome, String> {
    let result = result_of(reply, "tools/call")?;
    let mut parts: Vec<String> = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => parts.push(
                    item.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                ),
                Some(other) => parts.push(format!("[unsupported content of type {other:?}]")),
                None => parts.push("[unsupported content]".to_owned()),
            }
        }
    }
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(CallOutcome {
        text: parts.join("\n"),
        is_error,
    })
}

/// The structured half of a call's answer, when the server sent one. It is
/// passed through as data: the model reads it, nothing here interprets it.
pub(crate) fn read_structured(reply: &Value) -> Option<Value> {
    result_of(reply, "tools/call")
        .ok()?
        .get("structuredContent")
        .cloned()
}
