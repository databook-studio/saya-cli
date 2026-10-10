//! Direct overlap tests for the streaming [`Redactor`]: the refusal of
//! secrets that collide with the pattern-pass private-key marker and the
//! whole-value proofs for nested and split known values — the cases a
//! process-level test cannot isolate. Plain string fixtures only — no real
//! secrets.

use super::Redactor;

/// The longer value of the overlapping pair: the shorter value `abcd` with
/// the alphabet tail `QRSTUVWXYZ` appended, so the shorter value is a
/// strict prefix of it.
fn long_value() -> String {
    "abcd".to_string() + "QRSTUVWXYZ"
}

/// A bare occurrence of [`long_value`] surrounded by benign text.
fn bare_occurrence() -> Vec<u8> {
    let mut data = b"before abcd".to_vec();
    data.extend_from_slice(b"QRSTUVWXYZ after");
    data
}

#[test]
fn a_secret_inside_the_private_key_marker_is_refused() {
    let redactor = Redactor::new(&["private key".to_string()]);
    assert!(
        redactor.is_refused(),
        "a value the pattern-pass marker contains must be refused"
    );
}

#[test]
fn a_secret_carrying_the_private_key_marker_is_refused() {
    let redactor = Redactor::new(&["x [redacted private key] y".to_string()]);
    assert!(
        redactor.is_refused(),
        "a value containing the pattern-pass marker must be refused"
    );
}

#[test]
fn a_bare_longer_occurrence_is_redacted_whole() {
    let mut redactor = Redactor::new(&["abcd".to_string(), long_value()]);
    let mut output = redactor.push(&bare_occurrence());
    output.extend_from_slice(&redactor.finish());
    assert_eq!(
        String::from_utf8(output).expect("redacted output is valid UTF-8"),
        "before [redacted] after",
        "the nested shorter match must not split the longer value's marker"
    );
}

#[test]
fn a_bare_longer_occurrence_is_redacted_whole_in_reverse_order() {
    let mut redactor = Redactor::new(&[long_value(), "abcd".to_string()]);
    let mut output = redactor.push(&bare_occurrence());
    output.extend_from_slice(&redactor.finish());
    assert_eq!(
        String::from_utf8(output).expect("redacted output is valid UTF-8"),
        "before [redacted] after",
        "declaration order must not decide which match survives"
    );
}

/// The longer value split across two pushes: the holdback (>= 1024 bytes)
/// emits nothing before `finish`, so the carried text is re-redacted whole
/// once the value completes.
#[test]
fn a_value_split_across_pushes_is_redacted_whole() {
    let mut redactor = Redactor::new(&["abcd".to_string(), long_value()]);
    let mut output = redactor.push(b"abcdQRST");
    output.extend_from_slice(&redactor.push(b"UVWXYZ"));
    output.extend_from_slice(&redactor.finish());
    assert_eq!(
        String::from_utf8(output).expect("redacted output is valid UTF-8"),
        "[redacted]",
        "the holdback must carry the split value until it completes, then emit one marker"
    );
}

#[test]
fn a_value_spanning_a_marker_boundary_is_refused() {
    assert!(Redactor::new(&["acted] X".to_string()]).is_refused());
    assert!(Redactor::new(&["abc[".to_string()]).is_refused());
    assert!(!Redactor::new(&["abcd1234".to_string()]).is_refused());
}

#[test]
fn marker_synthesis_example_is_unreachable() {
    assert!(Redactor::new(&["abcd".to_string(), "acted]X".to_string()]).is_refused());
}

#[test]
fn an_unbounded_overlap_chain_seals_the_redactor() {
    let mut redactor = Redactor::new(&["aaaa".to_string()]);
    let chunk = vec![b'a'; 8192];
    for _ in 0..16 {
        redactor.push(&chunk);
    }
    assert!(redactor.is_refused());
    assert!(redactor.push(&chunk).is_empty());
    assert!(redactor.finish().is_empty());
}

#[test]
fn a_long_benign_stream_is_not_sealed() {
    let mut redactor = Redactor::new(&["aaaa".to_string()]);
    let chunk = vec![b'z'; 8192];
    let mut total = 0;
    for _ in 0..200 {
        total += redactor.push(&chunk).len();
    }
    assert!(!redactor.is_refused());
    total += redactor.finish().len();
    assert_eq!(total, 200 * 8192);
}
