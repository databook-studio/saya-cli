//! Whole-string behaviour of `scrub_secrets` against overlapping, adjacent,
//! nested, self-overlapping, and multibyte secret occurrences. Every test
//! asserts the exact scrubbed string, never a substring probe.

use super::scrub_secrets;

#[test]
fn a_nested_shorter_secret_leaves_no_tail_of_the_longer_one() {
    let out = scrub_secrets(
        "xx abcdQRSTUVWXYZ yy",
        &["abcd".to_owned(), "abcdQRSTUVWXYZ".to_owned()],
    );
    assert_eq!(out, "xx [redacted] yy");
}

#[test]
fn a_longer_secret_declared_first_scrubs_the_same_text() {
    let out = scrub_secrets(
        "xx abcdQRSTUVWXYZ yy",
        &["abcdQRSTUVWXYZ".to_owned(), "abcd".to_owned()],
    );
    assert_eq!(out, "xx [redacted] yy");
}

#[test]
fn partially_overlapping_secrets_merge_into_one_marker() {
    let out = scrub_secrets("abcdef", &["abcd".to_owned(), "cdef".to_owned()]);
    assert_eq!(out, "[redacted]");
}

#[test]
fn self_overlapping_occurrences_of_one_secret_merge_into_one_marker() {
    let out = scrub_secrets("aaaaaa", &["aaaa".to_owned()]);
    assert_eq!(out, "[redacted]");
}

#[test]
fn adjacent_non_overlapping_occurrences_keep_one_marker_each() {
    let out = scrub_secrets("abcdabcd", &["abcd".to_owned()]);
    assert_eq!(out, "[redacted][redacted]");
}

#[test]
fn a_multibyte_secret_between_text_is_scrubbed_exactly() {
    let out = scrub_secrets("xhéllo!", &["héllo".to_owned()]);
    assert_eq!(out, "x[redacted]!");
}

#[test]
fn overlapping_multibyte_matches_merge_without_panicking() {
    let out = scrub_secrets("xhélloéy", &["héllo".to_owned(), "élloé".to_owned()]);
    assert_eq!(out, "x[redacted]y");
}

#[test]
fn an_empty_secret_is_skipped_and_unmatched_text_is_returned_unchanged() {
    let out = scrub_secrets("keep me", &["".to_owned(), "absent".to_owned()]);
    assert_eq!(out, "keep me");
}

#[test]
fn a_secret_inside_surrounding_text_keeps_the_neighbors() {
    let out = scrub_secrets("abcde", &["bcde".to_owned()]);
    assert_eq!(out, "a[redacted]");
}
