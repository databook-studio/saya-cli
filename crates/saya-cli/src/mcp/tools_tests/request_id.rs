//! The actual request id's share of the wire measure (D9): the envelope
//! reserves room for the id a client actually sent, a large id pushes an
//! admitted result past the bound into a refusal, and an id that leaves no
//! room refuses even a rowless result.

use rmcp::model::{CallToolResult, RequestId};
use serde_json::json;

use super::super::{
    RESPONSE_ENVELOPE_HEADROOM_BYTES, bounded_built_result, response_envelope_bytes,
    response_wire_bytes,
};
use super::policy;
use crate::mcp::policy::MAX_RESPONSE_BYTES;

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
