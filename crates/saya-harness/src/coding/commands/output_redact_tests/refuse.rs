//! Refusal, overlap, and bound tests: unsound values are sealed out,
//! overlapping values are all destroyed, and the size bound holds.

use super::{MAX_SECRET_BYTES, MIN_SECRET_BYTES, REDACTION_MARKER_TEXT, Redactor};
use crate::runner::output::redact_chunk;

/// (e) A value that sits inside the marker, one that contains the marker,
/// and one shorter than 4 bytes make the redactor refused: the writer must
/// seal the store, so push/finish hand back nothing.
#[test]
fn unsound_values_are_refused_and_sealed() {
    for secret in [
        "redacted".to_string(),
        "x[redacted]y".to_string(),
        "abc".to_string(),
        String::new(),
    ] {
        let mut redactor = Redactor::new(std::slice::from_ref(&secret));
        assert!(redactor.is_refused(), "{secret:?} was accepted");
        assert!(redactor.push(b"any output at all").is_empty());
        assert!(redactor.finish().is_empty());
    }
    assert_eq!(MIN_SECRET_BYTES, 4);
}

/// (f) Two known values overlapping in one stream are both destroyed:
/// neither value, in either registry order, survives in the output.
#[test]
fn overlapping_values_are_both_destroyed() {
    let first = format!("AAAA{}", "0123456789");
    let second = format!("{}BBBB", "0123456789");
    let stream = format!("AAAA{}BBBB", "0123456789");
    for secrets in [
        &[first.clone(), second.clone()],
        &[second.clone(), first.clone()],
    ] {
        let mut redactor = Redactor::new(secrets);
        let mut emitted = Vec::new();
        emitted.extend(redactor.push(stream.as_bytes()));
        emitted.extend(redactor.finish());
        let text = String::from_utf8(emitted).expect("emitted bytes are valid UTF-8");
        assert!(!text.contains(first.as_str()), "{first:?} survived");
        assert!(!text.contains(second.as_str()), "{second:?} survived");
        assert!(text.contains(REDACTION_MARKER_TEXT));
    }
}

/// (g) A value longer than 4096 bytes makes the redactor refused; exactly
/// 4096 bytes is still accepted and sizes the holdback to it.
#[test]
fn overlong_value_is_refused() {
    let over = "S".repeat(MAX_SECRET_BYTES + 1);
    let mut redactor = Redactor::new(&[over]);
    assert!(redactor.is_refused());
    assert!(redactor.push(b"w").is_empty());
    assert!(redactor.finish().is_empty());

    let max = "S".repeat(MAX_SECRET_BYTES);
    let redactor = Redactor::new(&[max]);
    assert!(!redactor.is_refused());
    assert_eq!(
        redactor.holdback(),
        MAX_SECRET_BYTES - 1 + super::MIN_KEEP + super::KEEP_SLACK
    );
}

/// (i) The private marker literal equals the marker the runner gate
/// writes: redacting a known value through crate::runner::output's
/// redact_chunk yields output that contains exactly that literal.
#[test]
fn marker_literal_matches_the_runner_gate() {
    let secret = "SAYA-UNIT-TEST-SECRET-VALUE".to_string();
    let input = format!("value={secret}");
    let redacted = redact_chunk(&input, &[secret]);
    let expected = format!("value={}", REDACTION_MARKER_TEXT);
    assert_eq!(redacted, expected, "marker literals drifted apart");
}
