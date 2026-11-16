//! A tiny stdio MCP server for the client's tests.
//!
//! A real server would be another process to build and a second protocol
//! implementation to trust. This one answers the three requests the client
//! makes and can be told to misbehave in each of the ways the client's bounds
//! exist for: a hanging server, one that exits at once, one whose descriptions
//! or schemas are oversized, one that lists in pages, one that floods a frame,
//! one that sends a notification before every reply.
//!
//! It is an example target, so `cargo test` builds it beside the test binary
//! and the client's tests run it as a real child process on every platform.
//! `FIXTURE_MODE` selects the misbehaviour.

use std::io::{self, BufRead, Write};
use std::time::Duration;

const TOOLS_OK: &str = r#"{"tools":[{"name":"echo","description":"Echo the call back.","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}},{"name":"fail","description":"Report the call as failed.","inputSchema":{"type":"object"}},{"name":"secret","description":"Print the token this server was handed.","inputSchema":{"type":"object"}}]}"#;
const TOOLS_BAD_NAMES: &str = r#"{"tools":[{"name":"has space","description":"d","inputSchema":{"type":"object"}},{"name":"ok_name","description":"d","inputSchema":{"type":"object"}}]}"#;

fn main() {
    let mode = std::env::var("FIXTURE_MODE").unwrap_or_else(|_| "ok".to_owned());
    if mode == "exit" {
        std::process::exit(3);
    }
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut cursor_done = false;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let id = request_id(&line);
        if line.contains("\"method\":\"initialize\"") {
            if mode == "hang" {
                std::thread::sleep(Duration::from_secs(60));
                continue;
            }
            reply(
                &mut out,
                &id,
                r#"{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}"#,
            );
        } else if line.contains("\"method\":\"notifications/initialized\"") {
        } else if line.contains("\"method\":\"tools/list\"") {
            list(&mode, &id, &mut cursor_done, &mut out);
        } else if line.contains("\"method\":\"tools/call\"") {
            call(&mode, &line, &id, &mut out);
        } else if line.contains("\"method\":\"notifications/") {
        } else {
            write_line(
                &mut out,
                &format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{{\"code\":-32601,\
                     \"message\":\"unknown method\"}}}}"
                ),
            );
        }
    }
}

/// The `id` a request carries, as the digits the client sent.
fn request_id(line: &str) -> String {
    let Some(start) = line.find("\"id\":") else {
        return "null".to_owned();
    };
    let digits: String = line[start + 5..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        "null".to_owned()
    } else {
        digits
    }
}

/// The tool name a `tools/call` request names.
fn tool_name(line: &str) -> String {
    let Some(start) = line.find("\"name\":\"") else {
        return String::new();
    };
    let rest = &line[start + 8..];
    rest[..rest.find('"').unwrap_or(rest.len())].to_owned()
}

/// Answers one `tools/list`, including the modes that change the listing.
fn list(mode: &str, id: &str, cursor_done: &mut bool, out: &mut impl Write) {
    match mode {
        "huge_frame" => {
            // One line far past any frame cap: the client must discard it as
            // it arrives rather than buffer it.
            out.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":")
                .expect("stdout is writable");
            out.write_all(id.as_bytes()).expect("stdout is writable");
            out.write_all(b",\"result\":{\"tools\":[{\"name\":\"")
                .expect("stdout is writable");
            out.write_all(&vec![b'x'; 8_000_000])
                .expect("stdout is writable");
            out.write_all(b"\"}]}}\n").expect("stdout is writable");
            out.flush().expect("stdout is writable");
        }
        "many_tools" => {
            let mut result = String::from("{\"tools\":[");
            for index in 0..200 {
                if index > 0 {
                    result.push(',');
                }
                result.push_str(&format!(
                    "{{\"name\":\"tool_{index}\",\"description\":\"d\",\
                     \"inputSchema\":{{\"type\":\"object\"}}}}"
                ));
            }
            result.push_str("]}");
            reply(out, id, &result);
        }
        "paginate" if !*cursor_done => {
            *cursor_done = true;
            reply(
                out,
                id,
                r#"{"tools":[{"name":"first","description":"d","inputSchema":{"type":"object"}}],"nextCursor":"page-2"}"#,
            );
        }
        "big_description" => {
            let description = "d".repeat(4_000);
            reply(
                out,
                id,
                &format!(
                    "{{\"tools\":[{{\"name\":\"echo\",\"description\":\"{description}\",\
                     \"inputSchema\":{{\"type\":\"object\"}}}}]}}"
                ),
            );
        }
        "big_schema" => {
            let pad = "s".repeat(64 * 1024);
            reply(
                out,
                id,
                &format!(
                    "{{\"tools\":[{{\"name\":\"echo\",\"description\":\"fine\",\
                     \"inputSchema\":{{\"type\":\"object\",\"pad\":\"{pad}\"}}}}]}}"
                ),
            );
        }
        "bad_names" => reply(out, id, TOOLS_BAD_NAMES),
        _ => reply(out, id, TOOLS_OK),
    }
}

/// Answers one `tools/call`.
fn call(mode: &str, line: &str, id: &str, out: &mut impl Write) {
    let tool = tool_name(line);
    let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
    if mode == "notify_first" {
        write_line(
            out,
            r#"{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}}"#,
        );
    }
    match tool.as_str() {
        "fail" => reply(
            out,
            id,
            r#"{"content":[{"type":"text","text":"the tool refused"}],"isError":true}"#,
        ),
        "secret" => {
            let token = std::env::var("FIXTURE_TOKEN").unwrap_or_else(|_| "none".to_owned());
            reply(
                out,
                id,
                &format!(
                    "{{\"content\":[{{\"type\":\"text\",\"text\":\"token is {token}\"}}],\
                     \"isError\":false}}"
                ),
            );
        }
        _ => reply(
            out,
            id,
            &format!(
                "{{\"content\":[{{\"type\":\"text\",\"text\":\"echo {escaped}\"}}],\
                 \"isError\":false}}"
            ),
        ),
    }
}

/// Writes one successful reply.
fn reply(out: &mut impl Write, id: &str, result: &str) {
    write_line(
        out,
        &format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{result}}}"),
    );
}

/// Writes one line and flushes it, so the client sees the reply immediately.
fn write_line(out: &mut impl Write, line: &str) {
    out.write_all(line.as_bytes()).expect("stdout is writable");
    out.write_all(b"\n").expect("stdout is writable");
    out.flush().expect("stdout is writable");
}
