use super::projection::{self, Measurement, Messages};
use crate::ChatMessage;

fn measured(message: &ChatMessage) -> usize {
    match projection::measure(
        "model",
        Messages::Slice(std::slice::from_ref(message)),
        &[],
        usize::MAX,
    ) {
        Measurement::Fits(bytes) => bytes,
        Measurement::Exceeded => panic!("the projection cap was not reached"),
        Measurement::Serialization => panic!("the projection serialized"),
    }
}

/// Replayed reasoning is really sent on the routes that accept it, so the
/// request projection must measure it. A message that serializes without it
/// would understate the request the hard context cap bounds.
#[test]
fn replayed_reasoning_is_measured_in_the_request_projection() {
    let plain = ChatMessage::text("assistant", "answer");
    let trace = "r".repeat(4096);
    let with_reasoning = ChatMessage::text("assistant", "answer")
        .with_reasoning_trace(Some(trace.clone()), Some("signature".into()));

    let plain_bytes = measured(&plain);
    let reasoned_bytes = measured(&with_reasoning);
    assert!(
        reasoned_bytes >= plain_bytes + trace.len(),
        "the trace must be counted: {reasoned_bytes} vs {plain_bytes}"
    );
}
