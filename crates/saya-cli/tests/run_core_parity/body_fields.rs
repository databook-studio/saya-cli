//! One request body's parts, in the form two requests are compared in.
//!
//! The baseline states what a surface sends as whole values: the tool
//! definitions, the system messages, and the messages of the round trip. Each
//! is compared as its compact serialised form, whose object keys are sorted, so
//! two requests that sent the same content compare equal whatever their wire
//! order was.

/// The array at `key`, or an empty slice when the body states none.
pub(super) fn list<'a>(body: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    body[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The compact serialised form of one value; object keys are sorted, so two
/// requests that sent the same content compare equal whatever their wire
/// order.
pub(super) fn serial(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a request body serialises")
}

/// Every message of the body, serialised.
pub(super) fn messages(body: &serde_json::Value) -> Vec<Vec<u8>> {
    list(body, "messages").iter().map(serial).collect()
}

/// Every `system` message of the body, serialised.
pub(super) fn system(body: &serde_json::Value) -> Vec<Vec<u8>> {
    list(body, "messages")
        .iter()
        .filter(|message| message["role"] == "system")
        .map(serial)
        .collect()
}
