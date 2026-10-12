//! Stream-shape tests: invalid bytes never stall, finish flushes the
//! carry redacted, and emission is delayed by exactly both holdbacks.

use super::{REDACTION_MARKER_TEXT, Redactor, assert_no_long_fragment, benign, secret64};

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
