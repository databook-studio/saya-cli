use super::{COUNT_WORDS, COVERAGE_WORDS, PHASE_WORDS, full_snapshot};
use crate::local_metrics_format::format_snapshot;

/// (a) A fully-fed collector prints every static label exactly once, in the
/// stable phase → count → coverage slot order: the printed slot-label
/// sequences equal the expected tables, proving the label table is aligned
/// with the index functions.
#[test]
fn every_static_label_prints_exactly_once_in_a_stable_order() {
    let output = format_snapshot(&full_snapshot());
    let slot_labels = |kind: &str| -> Vec<&str> {
        output
            .lines()
            .filter_map(|line| line.strip_prefix(kind))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect()
    };
    assert_eq!(
        slot_labels("phase "),
        PHASE_WORDS,
        "phase labels in slot order"
    );
    assert_eq!(
        slot_labels("count "),
        COUNT_WORDS,
        "count labels in slot order"
    );
    assert_eq!(
        slot_labels("coverage "),
        COVERAGE_WORDS,
        "coverage labels in slot order"
    );
    assert!(
        output.starts_with("saya local metrics (fixed fields; not a benchmark)\n"),
        "the output opens with the fixed header: {output}"
    );
    assert!(
        output.contains("first_useful non_whitespace_text"),
        "the kept first-useful kind is named: {output}"
    );
    assert!(
        !output.contains("successful_tool"),
        "only the first useful record is kept: {output}"
    );
    assert!(
        output
            .lines()
            .any(|line| line.starts_with("unrecognized 0")),
        "the unrecognized counter is printed: {output}"
    );
}
