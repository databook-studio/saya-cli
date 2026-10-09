use super::{COUNT_WORDS, COVERAGE_WORDS, PHASE_WORDS, full_snapshot};
use crate::local_metrics_format::format_snapshot;

/// (d) The whole output is made only of the allowed label vocabulary, digits,
/// and punctuation: no request, result, error, or profile string can reach the
/// formatter by construction, and this check fails if that ever changes.
#[test]
fn the_output_is_only_labels_digits_and_punctuation() {
    const STRUCTURE: [&str; 25] = [
        "saya",
        "local",
        "metrics",
        "fixed",
        "fields",
        "not",
        "a",
        "benchmark",
        "phase",
        "count",
        "coverage",
        "first_useful",
        "unrecognized",
        "unavailable",
        "samples",
        "total_us",
        "max_us",
        "ok",
        "fail",
        "cancel",
        "unavail",
        "cached",
        "skipped",
        "reported",
        "missing",
    ];
    const KINDS: [&str; 3] = ["at_us", "non_whitespace_text", "successful_tool"];
    let vocabulary: Vec<&str> = PHASE_WORDS
        .into_iter()
        .chain(COUNT_WORDS)
        .chain(COVERAGE_WORDS)
        .chain(STRUCTURE)
        .chain(KINDS)
        .collect();
    let output = format_snapshot(&full_snapshot());
    assert!(
        output
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || " _=(;)\n".contains(c)),
        "only lowercase labels, digits, and known punctuation: {output}"
    );
    for token in output.split_whitespace() {
        let letters: String = token
            .chars()
            .filter(|c| c.is_ascii_lowercase() || *c == '_')
            .collect();
        assert!(
            letters.is_empty() || vocabulary.contains(&letters.as_str()),
            "every token is vocabulary or a number: {token:?} in {output}"
        );
    }
}
