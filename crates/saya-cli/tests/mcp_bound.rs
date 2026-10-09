//! The response bound holds on the wire (A922-5, D5; D9). `saya mcp serve`'s
//! `query` answer is narrowed by dropping whole rows until the FINAL reply
//! LINE — the JSON-RPC envelope with the client's ACTUAL request id wrapped
//! around the built result, plus the writer's newline — fits the 16 MiB
//! bound; rmcp 3.5.0's `CallToolResult::structured` duplicates the payload
//! into a text content block, so the wire carries roughly twice the payload,
//! and a large client-controlled id rides the envelope on top. `rows`,
//! `truncated`, and the evidence row counts stay consistent. The real binary
//! is driven over stdio pipes, like a real MCP client; stdin is held open
//! until the reply has arrived, because requests still pending at EOF are
//! dropped.
//!
//! Config isolation mirrors the sibling MCP tests: `--config`/`--connections`
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

use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// The published response bound (mcp/policy.rs), restated here so the test
/// fails loudly when the wire exceeds it.
const MAX_RESPONSE_BYTES: usize = 16_777_216;
/// One mebibyte of text per cell; ten cells ≈ 10 MiB of payload that the
/// payload-only check admitted whole while the wire carried ~2×.
const CELL: usize = 1_048_576;
const ROWS: u64 = 10;

include!("mcp_bound/fixture.rs");
include!("mcp_bound/server.rs");
include!("mcp_bound/base_wire_bound.rs");

/// The audit's reproduction SQL (R061-3): eight rows of a 1,040,000-char
/// cell — each cell rides just under the connector's 1 MiB cap, and the
/// duplicated text content block carries the built result to ~16.64 MB:
/// inside the old fixed-reserve measure (which admitted it whole) but over
/// the true reply line for any large request id.
const AUDIT_SQL: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n \
                         WHERE x<8) SELECT printf('%1040000s','x') AS cell FROM n";

/// The audit's reproduction `_meta` (R29-1): a `tools/call` that declares the
/// 2026-07-28 protocol inline, so rmcp 3.5.0 keeps the `resultType`
/// discriminator on the reply and the measured budget matches the wire with
/// no legacy slack. The client-capabilities key is required by the same
/// protocol draft for a request that declares its version inline.
fn audit_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

/// Every admitted request id must get a reply line within the bound (D9).
/// The three id shapes below exercise the envelope's real serialization:
/// plain ASCII (the audit's case), a quote/backslash id whose JSON escaping
/// doubles its wire size, and a non-ASCII id carried as UTF-8 bytes.
/// Assertions: the reply LINE (newline included) fits the bound, the answer
/// is narrowed — never refused — and `rows`/`row_count`/`truncated`/the
/// evidence counts all agree.
fn assert_reply_line_fits_and_stays_consistent(reply_line: &str) {
    assert!(
        reply_line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes",
        MAX_RESPONSE_BYTES,
        reply_line.len()
    );
    let reply: Value = serde_json::from_str(reply_line).expect("the reply line is JSON");
    assert_eq!(
        reply["result"]["isError"], false,
        "the oversized answer is narrowed and sent, not refused"
    );
    let payload = &reply["result"]["structuredContent"];
    assert_eq!(
        payload["truncated"], true,
        "the answer is marked truncated: {}",
        payload["note"]
    );
    let sent = payload["row_count"]
        .as_u64()
        .expect("row_count is a number");
    assert!(
        sent < 8,
        "rows were dropped to fit the wire: {sent} of the 8 the SQL returns"
    );
    assert_eq!(
        payload["rows"].as_array().expect("rows array").len() as u64,
        sent,
        "row_count matches the rows actually sent"
    );
    assert_eq!(
        payload["evidence"]["returned_rows"], sent,
        "the evidence reports the rows actually sent"
    );
    assert_eq!(
        payload["evidence"]["truncated"], true,
        "the evidence reports the narrowing"
    );
    assert!(
        payload["note"]
            .as_str()
            .unwrap_or_default()
            .contains("cut to"),
        "the narrowing is named: {}",
        payload["note"]
    );
}

include!("mcp_bound/request_id_shapes.rs");

include!("mcp_bound/truncation_and_meta.rs");

include!("mcp_bound/boundary_scan.rs");
