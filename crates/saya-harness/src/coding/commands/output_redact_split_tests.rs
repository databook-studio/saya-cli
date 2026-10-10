//! Split-overlap tests for the streaming [`Redactor`]: secret matches whose
//! text straddles `push` boundaries — partial overlaps, self-overlaps, and
//! pairs cut at the stage-a holdback line — must emit exactly one marker per
//! logical span. Plain string fixtures only — no real secrets.

use super::Redactor;

/// Build a redactor from `secrets`, concatenate the output of every push,
/// then append `finish`, returning the final text.
fn run(secrets: &[&str], pushes: &[Vec<u8>]) -> String {
    let owned: Vec<String> = secrets.iter().map(|secret| secret.to_string()).collect();
    let mut redactor = Redactor::new(&owned);
    let mut output = Vec::new();
    for push in pushes {
        output.extend_from_slice(&redactor.push(push));
    }
    output.extend_from_slice(&redactor.finish());
    String::from_utf8(output).expect("redacted output is valid UTF-8")
}

/// The shorter value `abcd` ends at the cut and its overlap partner `cdef`
/// starts before it: the two pushes together must produce one marker.
#[test]
fn partial_overlap_split_across_pushes_is_one_marker() {
    assert_eq!(
        run(&["abcd", "cdef"], &[b"abcd".to_vec(), b"ef".to_vec()]),
        "[redacted]"
    );
}

/// A value that overlaps itself, cut mid-match across pushes, must not
/// duplicate the marker for the re-processed tail.
#[test]
fn self_overlap_split_across_pushes_is_one_marker() {
    assert_eq!(
        run(&["xabcx"], &[b"xabcxabc".to_vec(), b"x".to_vec()]),
        "[redacted]"
    );
}

/// A nested pair whose longer value straddles the stage-a holdback cut
/// (>= 1024 bytes before the match) must be re-redacted whole after the cut.
#[test]
fn nested_pair_straddling_the_stage_a_cut_is_whole() {
    let prefix = "z".repeat(1100);
    let output = run(
        &["abcd", "abcdQRSTUVWXYZ"],
        &[format!("{prefix}abcdQRST").into_bytes(), b"UVWXYZ".to_vec()],
    );
    assert_eq!(output, format!("{prefix}[redacted]"));
    assert!(
        !output.contains("QRSTUVWXYZ"),
        "the split longer value must be redacted whole, not leaked in part"
    );
}

/// Two disjoint values split across pushes at the holdback cut must each
/// collapse to exactly one marker, preserving the separating text.
#[test]
fn disjoint_pair_split_across_pushes_is_redacted() {
    let prefix = "z".repeat(1100);
    let output = run(
        &["abcd1234", "wxyz5678"],
        &[
            format!("{prefix}abcd").into_bytes(),
            b"1234-wxyz".to_vec(),
            b"5678".to_vec(),
        ],
    );
    assert_eq!(output, format!("{prefix}[redacted]-[redacted]"));
}
