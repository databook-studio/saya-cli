//! Tests for the shared context-window thresholds: the warn/compact
//! percentages later slices read, and the utilisation helper the footer and
//! the warning trigger share so the two can never round differently.

use super::{CONTEXT_COMPACT_PERCENT, CONTEXT_WARN_PERCENT, context_utilisation_percent};

/// The owner-set thresholds: warn at 70%, compact at 95%. Pinned here so a
/// later slice cannot silently drift either number — or fork a second copy.
#[test]
fn the_owner_set_thresholds_are_warn_70_compact_95() {
    assert_eq!(CONTEXT_WARN_PERCENT, 70);
    assert_eq!(CONTEXT_COMPACT_PERCENT, 95);
}

/// No denominator means no percentage — the table's own principle ("a wrong
/// window is worse than no window") at the helper level: never divide by a
/// fiction, and never divide by zero (config rejects a zero window, so one
/// is invalid input, not a tiny window).
#[test]
fn no_window_or_a_zero_window_is_no_percentage_never_zero() {
    assert_eq!(context_utilisation_percent(Some(64_000), None), None);
    assert_eq!(context_utilisation_percent(Some(64_000), Some(0)), None);
}

/// Absence is not zero: no provider report of the input is no percentage,
/// never `0%` — a silent provider must not read as an empty context.
#[test]
fn no_input_report_is_no_percentage_never_zero_percent() {
    assert_eq!(context_utilisation_percent(None, Some(128_000)), None);
}

/// The rounding the footer already used: exact division, rounded to the
/// display precision, unclamped above 100% (an overflowed turn must read as
/// overflowed, not as exactly full).
#[test]
fn known_input_over_a_known_window_rounds_like_the_footer() {
    assert_eq!(
        context_utilisation_percent(Some(64_000), Some(128_000)),
        Some(50)
    );
    assert_eq!(
        context_utilisation_percent(Some(200_000), Some(128_000)),
        Some(156)
    );
    assert_eq!(context_utilisation_percent(Some(0), Some(128_000)), Some(0));
}
