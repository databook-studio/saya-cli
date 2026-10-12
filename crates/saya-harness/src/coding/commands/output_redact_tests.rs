//! Direct unit tests for the streaming [`Redactor`]: the cut-boundary and
//! byte-exact-carry proofs a process-level test cannot give. Plain
//! string/byte fixtures only — no real secrets.

use super::{
    KEEP_SLACK, MAX_SECRET_BYTES, MIN_KEEP, MIN_SECRET_BYTES, REDACTION_MARKER_TEXT, Redactor,
};

// This module is loaded through `#[path = "output_redact_tests.rs"]` from
// `output_redact.rs`, so it does not own a subdirectory: the child module
// files need explicit paths to live in `output_redact_tests/`.
#[path = "output_redact_tests/refuse.rs"]
mod refuse;

#[path = "output_redact_tests/split.rs"]
mod split;

#[path = "output_redact_tests/stream.rs"]
mod stream;

/// A 64-byte ASCII value on an alphabet disjoint from the benign `x`
/// filler and the marker, so a surviving fragment can only mean a redactor
/// defect.
fn secret64() -> String {
    let mut value = "0123456789".repeat(7);
    value.truncate(64);
    value
}

/// A benign prefix long enough that pushes actually emit past the cut.
fn benign(len: usize) -> Vec<u8> {
    vec![b'x'; len]
}

/// Asserts no char-boundary substring of `secret` of at least `min_len`
/// bytes appears in `text`. Sub-byte fragments shorter than `min_len` are
/// allowed: they cannot reconstruct the value and neighbor text may
/// legitimately survive around a marker.
fn assert_no_long_fragment(text: &str, secret: &str, min_len: usize, split: usize) {
    let bounds: Vec<usize> = secret
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(secret.len()))
        .collect();
    for start in &bounds {
        for end in &bounds {
            if end > start && end - start >= min_len {
                let fragment = &secret[*start..*end];
                assert!(
                    !text.contains(fragment),
                    "fragment {fragment:?} survived (split {split})"
                );
            }
        }
    }
}

/// Feeds `value` split across two pushes after a benign prefix that ends
/// the first push's window, drains everything, and returns the redacted
/// text plus the holdback size used.
fn fed_split(secret: &str, prefix_len: usize, split: usize) -> String {
    let secret = secret.to_string();
    let bytes = secret.as_bytes();
    let mut redactor = Redactor::new(std::slice::from_ref(&secret));
    let mut emitted = Vec::new();
    let mut first = benign(prefix_len);
    first.extend_from_slice(&bytes[..split]);
    emitted.extend(redactor.push(&first));
    emitted.extend(redactor.push(&bytes[split..]));
    emitted.extend(redactor.finish());
    String::from_utf8(emitted).expect("emitted bytes are valid UTF-8")
}
