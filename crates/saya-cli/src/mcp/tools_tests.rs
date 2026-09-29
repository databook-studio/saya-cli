//! Unit tests for the response-bound measure (A922-5, D5; D9): the bound is
//! checked against the FINAL reply line — rmcp 3.5.0's
//! `CallToolResult::structured` duplicates the payload into a text content
//! block, so the wire carries roughly twice the payload — plus the JSON-RPC
//! envelope with the ACTUAL request id, measured from rmcp's own
//! serialization. The wire behavior is pinned in `tests/mcp_bound.rs`
//! against the real binary.

use std::{collections::BTreeMap, path::PathBuf};

use rmcp::{
    RoleServer,
    model::{CallToolResult, RequestId, ServerResult},
    service::TxJsonRpcMessage,
};
use serde_json::json;

use super::{
    RESPONSE_ENVELOPE_HEADROOM_BYTES, bounded_built_result, bounded_result,
    response_envelope_bytes, response_wire_bytes, result_wire_bytes,
};
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

/// Request ids whose shapes stress the envelope's serialization: numeric,
/// the audit's 200,000-char ASCII id, an id whose JSON escaping doubles its
/// wire size, and a non-ASCII id carried as raw UTF-8.
fn id_cases() -> Vec<(&'static str, RequestId)> {
    vec![
        ("numeric", RequestId::Number(2)),
        ("ascii-200k", RequestId::String("i".repeat(200_000).into())),
        (
            "escaping",
            RequestId::String(format!("{}{}", "\"".repeat(100_000), "\\".repeat(100_000)).into()),
        ),
        ("non-ascii", RequestId::String("é".repeat(100_000).into())),
    ]
}

/// The envelope measure is rmcp's own wrapper, serialized with the actual id
/// exactly as the writer writes the reply line — escapes, non-ASCII, and the
/// trailing newline included — and it composes additively with the result
/// measure (D9).
#[test]
fn the_envelope_measure_is_the_exact_wrapper_rmcp_writes() {
    let built = CallToolResult::structured(json!({ "rows": ["x".repeat(64)] }));
    let built_bytes = serde_json::to_vec(&built).unwrap().len();
    for (label, id) in id_cases() {
        let line = serde_json::to_vec(&TxJsonRpcMessage::<RoleServer>::response(
            ServerResult::CallToolResult(built.clone()),
            id.clone(),
        ))
        .unwrap();
        assert_eq!(
            line.len(),
            response_wire_bytes(&built, &id),
            "the measure is the exact reply line for the {label} id"
        );
        assert_eq!(
            line.len(),
            built_bytes + response_envelope_bytes(&id),
            "envelope and result compose additively for the {label} id"
        );
    }
}

/// The envelope reserves room for the ACTUAL request id: it grows with the
/// id's wire size, and an id full of `"` and `\` costs double — its JSON
/// escaping rides the reply line too (D9).
#[test]
fn the_envelope_reserves_room_for_the_actual_request_id() {
    let floor = response_envelope_bytes(&RequestId::Number(2));
    let ascii = response_envelope_bytes(&RequestId::String("i".repeat(200_000).into()));
    let escaping = response_envelope_bytes(&RequestId::String(
        format!("{}{}", "\"".repeat(100_000), "\\".repeat(100_000)).into(),
    ));
    assert!(floor < 128, "a numeric id's envelope is tiny: {floor}");
    assert!(
        ascii > floor + 199_000,
        "the 200,000-char id rides the envelope: {ascii} vs {floor}"
    );
    assert!(
        escaping > ascii + 199_000,
        "escaping doubles the id's wire size: {escaping} vs {ascii}"
    );
}

/// A result the old fixed-reserve measure admitted is refused for a large
/// request id: the actual envelope pushes the reply line past the bound
/// (D9) — and the refusal that replaces it is itself a line that fits.
#[test]
fn a_result_fitting_the_old_reserve_is_refused_for_a_large_request_id() {
    let payload = json!({ "rows": ["x".repeat(8_330_000)] });
    let built = CallToolResult::structured(payload);
    let built_bytes = serde_json::to_vec(&built).unwrap().len();
    assert!(
        built_bytes + RESPONSE_ENVELOPE_HEADROOM_BYTES <= MAX_RESPONSE_BYTES,
        "setup: the fixed-reserve measure admits this result ({built_bytes} bytes)"
    );
    let id = RequestId::String("i".repeat(200_000).into());
    assert!(
        built_bytes + response_envelope_bytes(&id) > MAX_RESPONSE_BYTES,
        "setup: the actual envelope pushes this result past the bound"
    );

    let refusal = bounded_built_result(&policy(), built, &id)
        .expect("the refusal is a tool-level result, not a protocol error");
    match refusal {
        rmcp::model::CallToolResponse::Complete(result) => {
            assert_eq!(result.is_error, Some(true), "the refusal is a tool error");
            let rendered = serde_json::to_string(&result).unwrap();
            assert!(
                rendered.contains("response bound"),
                "the refusal names the bound: {rendered}"
            );
            assert!(
                policy().response_allowed(response_wire_bytes(&result, &id)),
                "the refusal itself fits the wire for this id"
            );
        }
        other => panic!("a bounded refusal is a complete result, not {other:?}"),
    }
}

/// A request id that alone leaves no room for even a rowless result is
/// refused: the budget saturates and the tool-level error goes out instead
/// of a fat line (D9). Such an id cannot pass the 1 MiB inbound line gate,
/// but the measure must stay total for any `&str` a client might send.
#[test]
fn a_request_id_that_leaves_no_room_refuses_even_a_rowless_result() {
    let id = RequestId::String("i".repeat(17 * 1_048_576).into());
    assert!(
        response_envelope_bytes(&id) >= MAX_RESPONSE_BYTES,
        "setup: the envelope alone leaves no room for a rowless result"
    );
    let refusal = bounded_built_result(&policy(), CallToolResult::structured(json!({})), &id)
        .expect("the refusal is a tool-level result, not a protocol error");
    match refusal {
        rmcp::model::CallToolResponse::Complete(result) => {
            assert_eq!(result.is_error, Some(true), "the refusal is a tool error");
        }
        other => panic!("a bounded refusal is a complete result, not {other:?}"),
    }
}
