//! Unit tests for the response-bound measure (A922-5, D5): the bound is
//! checked against the FINAL built result — rmcp 3.5.0's
//! `CallToolResult::structured` duplicates the payload into a text content
//! block, so the wire carries roughly twice the payload — plus the reserved
//! JSON-RPC envelope headroom. The wire behavior is pinned in
//! `tests/mcp_bound.rs` against the real binary.

use std::{collections::BTreeMap, path::PathBuf};

use rmcp::model::CallToolResult;
use serde_json::json;

use super::{RESPONSE_ENVELOPE_HEADROOM_BYTES, bounded_result, result_wire_bytes};
use crate::config::runtime::RuntimeConfig;
use crate::mcp::policy::{MAX_RESPONSE_BYTES, ServePolicy};
use saya_config::{CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

fn policy() -> ServePolicy {
    let connections =
        ConnectionsFile::from_toml("[profiles.p]\ntype = 'sqlite'\npath = 'p.sqlite3'\n").unwrap();
    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(ConfigFile::from_toml("").unwrap())
            .with_cli(CliOverrides::default()),
    )
    .unwrap();
    ServePolicy::resolve(
        &RuntimeConfig {
            resolved,
            connections,
            config_path: None,
            connections_path: None,
            cache_scope: PathBuf::new(),
            investigations_root: crate::config::runtime::temp_investigations_root(),
            secret_values: BTreeMap::new(),
        },
        &[],
        None,
    )
    .unwrap()
}

/// A payload that fits the bound on its own but doubles past it once built
/// into a structured result.
fn near_bound_payload() -> serde_json::Value {
    // 9 MiB of text: the payload alone stays under the bound; the duplicated
    // text content block pushes the built result past it.
    json!({ "rows": ["x".repeat(9 * 1_048_576)] })
}

/// The measure covers the built result, not just the payload: rmcp's
/// `structured` duplicates the payload into a text content block, so the
/// wire carries roughly twice the payload plus the reserved envelope
/// headroom.
#[test]
fn the_wire_measure_covers_the_built_result_not_just_the_payload() {
    let payload = near_bound_payload();
    let payload_bytes = serde_json::to_vec(&payload).unwrap().len();
    assert!(
        payload_bytes <= MAX_RESPONSE_BYTES,
        "setup: the payload alone fits ({payload_bytes} bytes)"
    );
    let result = CallToolResult::structured(payload.clone());
    let built_bytes = serde_json::to_vec(&result).unwrap().len();
    assert!(
        built_bytes > 2 * payload_bytes,
        "setup: the text block duplicates the payload ({built_bytes} vs {payload_bytes} bytes)"
    );
    assert_eq!(
        result_wire_bytes(&result),
        built_bytes + RESPONSE_ENVELOPE_HEADROOM_BYTES,
        "the measure is the serialized result plus the envelope headroom"
    );
    assert!(
        result_wire_bytes(&result) > MAX_RESPONSE_BYTES,
        "the built result is past the bound where the payload alone was not"
    );
}

/// A built result past the bound is refused as a tool-level error even when
/// the payload's own serialization fits (D5): never sent fat.
#[test]
fn an_oversized_built_result_is_refused_even_when_the_payload_fits() {
    let refusal = bounded_result(&policy(), near_bound_payload())
        .expect("the refusal is a tool-level result, not a protocol error");
    match refusal {
        rmcp::model::CallToolResponse::Complete(result) => {
            assert_eq!(result.is_error, Some(true), "the refusal is a tool error");
            let rendered = serde_json::to_string(&result).unwrap();
            assert!(
                rendered.contains("response bound"),
                "the refusal names the bound: {rendered}"
            );
        }
        other => panic!("a bounded refusal is a complete result, not {other:?}"),
    }
}
