//! Cut-boundary tests: values split across pushes at every byte offset
//! are held whole and redacted exactly once.

use super::{
    REDACTION_MARKER_TEXT, Redactor, assert_no_long_fragment, benign, fed_split, secret64,
};

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
    let secret = "AAA\u{1F980}BBB".to_string();
    let _bytes = secret.as_bytes();
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
