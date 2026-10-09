//! Stdio contract for `saya mcp serve` (ADR 0008, task Db). The real binary
//! is driven over pipes, like a real MCP client: newline-delimited JSON-RPC
//! on stdout and nothing else, a toolset bounded to the startup allowlist
//! (names + dialects, never paths, hosts, or identities), row-returning tools
//! listed only when data sharing is allowed, the same read-only safety gate
//! the CLI uses, a JSON-RPC error — not a crash — for an oversized request
//! line, and exit 0 on stdin EOF.
//!
//! Config isolation mirrors the sibling CLI tests: `--config`/`--connections`
//! point at a temp fixture and `SAYA_STATE_DB`/`SAYA_INVESTIGATIONS_DIR` at
//! temp paths, so no machine-level profile or store leaks into the
//! assertions.

use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use serde_json::Value;

const PROTOCOL_VERSION: &str = "2025-06-18";

include!("mcp_stdio/fixture.rs");
include!("mcp_stdio/server.rs");
fn initialize_request() -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{}},"clientInfo":{{"name":"contract-test","version":"0"}}}}}}"#
    )
}

fn initialized_notification() -> &'static str {
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
}

const SLOW_SQL: &str = "WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c) \
                        SELECT sum(x) FROM c";

include!("mcp_stdio/stdout_framing.rs");

include!("mcp_stdio/startup_and_allowlist.rs");

include!("mcp_stdio/query_policy.rs");

include!("mcp_stdio/cancellation.rs");

include!("mcp_stdio/investigation_replay.rs");

include!("mcp_stdio/profile_allowlist.rs");
