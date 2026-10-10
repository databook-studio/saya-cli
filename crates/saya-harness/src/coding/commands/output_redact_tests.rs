//! Direct unit tests for the streaming [`Redactor`]: the cut-boundary and
//! byte-exact-carry proofs a process-level test cannot give. Plain
//! string/byte fixtures only — no real secrets.

use super::{MAX_SECRET_BYTES, MIN_SECRET_BYTES, REDACTION_MARKER_TEXT, Redactor};
use crate::runner::output::redact_chunk;

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

/// (a) A 64-byte value split at every offset 1..len-1 across two pushes,
/// the first half ending the first push's window: the concatenated
/// emissions plus finish never contain the value nor any fragment of
/// length >= 8, and contain the marker exactly once.
#[test]
fn split_value_is_never_emitted_at_any_offset() {
    let secret = secret64();
    for split in 1..secret.len() {
        let text = fed_split(&secret, 2048, split);
        assert!(
            !text.contains(secret.as_str()),
            "value leaked at split {split}"
        );
        assert_no_long_fragment(&text, &secret, 8, split);
        assert_eq!(
            text.matches(REDACTION_MARKER_TEXT).count(),
            1,
            "marker count wrong at split {split}"
        );
    }
}

/// (b) The same, but the benign prefix is exactly the holdback long so the
/// emission cut lands exactly where the value begins and then inside the
/// value's span across the carry: the cut must still stop short of the
/// un-redacted bytes.
#[test]
fn value_straddling_the_cut_is_held_whole() {
    let secret = secret64();
    let probe = Redactor::new(std::slice::from_ref(&secret));
    let keep = probe.holdback();
    for split in 1..secret.len() {
        let text = fed_split(&secret, keep, split);
        assert!(
            !text.contains(secret.as_str()),
            "value leaked at split {split}"
        );
        assert_no_long_fragment(&text, &secret, 8, split);
        assert_eq!(
            text.matches(REDACTION_MARKER_TEXT).count(),
            1,
            "marker count wrong at split {split}"
        );
    }
    // The cut also lands inside the (replaced) value span when the value
    // sits mid-window with a suffix sized so len - keep falls inside it:
    // the marker may be split across an emission boundary, but the
    // concatenated output must still carry it exactly once and intact.
    let mut redactor = Redactor::new(std::slice::from_ref(&secret));
    let mut emitted = Vec::new();
    let mut stream = benign(keep);
    stream.extend_from_slice(secret.as_bytes());
    stream.extend_from_slice(&benign(keep - 8));
    let mid = stream.len() / 2;
    emitted.extend(redactor.push(&stream[..mid]));
    emitted.extend(redactor.push(&stream[mid..]));
    emitted.extend(redactor.finish());
    let text = String::from_utf8(emitted).expect("emitted bytes are valid UTF-8");
    assert!(!text.contains(secret.as_str()));
    assert_no_long_fragment(&text, &secret, 8, 0);
    assert_eq!(text.matches(REDACTION_MARKER_TEXT).count(), 1);
}

/// (c) A 4-byte UTF-8 character (U+1F980) inside a known value, split at
/// each of its 3 internal byte offsets across two pushes: the held
/// incomplete sequence is never lossily decoded, so the value assembles
/// whole and is redacted — no U+FFFD, no fragment >= 8.
#[test]
fn four_byte_char_split_inside_is_still_redacted() {
    let secret = format!("AAA\u{1F980}BBB");
    let bytes = secret.as_bytes();
    for split in [4usize, 5, 6] {
        let text = fed_split(&secret, 2048, split);
        assert!(
            !text.contains(secret.as_str()),
            "value leaked at split {split}"
        );
        assert!(
            !text.contains('\u{FFFD}'),
            "held bytes were mangled at split {split}"
        );
        assert_no_long_fragment(&text, &secret, 8, split);
        assert_eq!(
            text.matches(REDACTION_MARKER_TEXT).count(),
            1,
            "marker count wrong at split {split}"
        );
    }
}

/// (d) Invalid bytes (0xFF) never stall output and never panic: emissions
/// still flow past the cut, every stream stays valid UTF-8 with U+FFFD in
/// place of the bad sequence, and a complete value is still redacted.
#[test]
fn invalid_bytes_never_stall_or_panic() {
    let secret = secret64();
    let mut redactor = Redactor::new(std::slice::from_ref(&secret));
    let mut emitted = Vec::new();
    let mut first = benign(2048);
    first.extend_from_slice(&[0xFF, b'j', 0xFF]);
    emitted.extend(redactor.push(&first));
    assert!(!emitted.is_empty(), "output stalled on invalid bytes");
    emitted.extend(redactor.push(&[0xFF, b'k', 0xFF]));
    emitted.extend(redactor.push(secret.as_bytes()));
    emitted.extend(redactor.push(&[0xFF, b't']));
    emitted.extend(redactor.finish());
    let text = String::from_utf8(emitted).expect("emitted bytes are valid UTF-8");
    assert!(text.contains('\u{FFFD}'), "invalid bytes vanished");
    assert!(!text.contains(secret.as_str()));
    assert!(text.contains(REDACTION_MARKER_TEXT));
    // A 0xC3 held as an incomplete sequence and completed by an invalid
    // byte across pushes: one U+FFFD, no panic, output keeps flowing.
    let mut redactor = Redactor::new(&["m".repeat(16)]);
    let mut emitted = Vec::new();
    emitted.extend(redactor.push(&[0xC3]));
    emitted.extend(redactor.push(&[0xFF]));
    emitted.extend(redactor.push(b"tail"));
    emitted.extend(redactor.finish());
    let text = String::from_utf8(emitted).expect("emitted bytes are valid UTF-8");
    assert!(text.contains('\u{FFFD}'));
    assert!(text.contains("tail"));
}

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
        let mut redactor = Redactor::new(&[secret.clone()]);
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

/// (h) finish() flushes the carry fully redacted: the redacted value that
/// sat in the held tail is emitted by finish, nothing is dropped, and a
/// second finish hands back nothing.
#[test]
fn finish_flushes_the_carry_fully_redacted() {
    let secret = secret64();
    let mut redactor = Redactor::new(std::slice::from_ref(&secret));
    let keep = redactor.holdback();
    let mut emitted = Vec::new();
    emitted.extend(redactor.push(&benign(keep + 512)));
    emitted.extend(redactor.push(secret.as_bytes()));
    assert!(
        emitted.iter().all(|byte| *byte == b'x'),
        "an emission carried value bytes"
    );
    let tail = redactor.finish();
    assert!(!tail.is_empty(), "the carry was dropped instead of flushed");
    assert!(
        tail.windows(REDACTION_MARKER_TEXT.len())
            .any(|window| window == REDACTION_MARKER_TEXT.as_bytes()),
        "finish dropped the redacted value"
    );
    assert!(
        redactor.finish().is_empty(),
        "a second finish kept bytes back"
    );
    emitted.extend(tail);
    let text = String::from_utf8(emitted).expect("emitted bytes are valid UTF-8");
    assert!(!text.contains(secret.as_str()));
    assert_no_long_fragment(&text, &secret, 8, 0);
    assert_eq!(text.matches(REDACTION_MARKER_TEXT).count(), 1);
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

/// (j) The two-stage holdback delays emission by exactly `holdback()`:
/// `holdback() + 100` benign bytes emit exactly 100, finish flushes the rest.
#[test]
fn emission_is_delayed_by_both_holdbacks() {
    let secret = secret64();
    let mut redactor = Redactor::new(std::slice::from_ref(&secret));
    let mut emitted = redactor.push(&benign(redactor.holdback() + 100));
    assert!(emitted.len() == 100 && emitted.iter().all(|byte| *byte == b'x'));
    emitted.extend(redactor.finish());
    assert_eq!(emitted.len(), redactor.holdback() + 100);
    assert!(emitted.iter().all(|byte| *byte == b'x'));
}
