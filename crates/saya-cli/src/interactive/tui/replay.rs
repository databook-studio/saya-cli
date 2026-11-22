//! Time as the rows state it.
//!
//! A saved-session list and a recovery report say when something happened in
//! the same compact words ("3m ago"), so both read this one function. The old
//! surface's resume replay (`history_blocks`) went with it in X1: nothing builds
//! its own transcript from a saved session any more.

/// Formats a millisecond age as a compact relative time (e.g. "3h ago").
pub(crate) fn relative_time(delta_ms: u128) -> String {
    let secs = delta_ms / 1000;
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::relative_time;

    /// Under a minute is "just now"; minutes, hours, and days each round down
    /// to the largest unit that fits, and the day boundary is exact.
    #[test]
    fn an_age_reads_in_the_largest_unit_that_fits() {
        assert_eq!(relative_time(0), "just now");
        assert_eq!(relative_time(59_999), "just now", "under a minute");
        assert_eq!(relative_time(60_000), "1m ago");
        assert_eq!(relative_time(3_599_999), "59m ago");
        assert_eq!(relative_time(3_600_000), "1h ago");
        assert_eq!(relative_time(86_400_000), "1d ago");
        assert_eq!(relative_time(3 * 86_400_000), "3d ago");
    }
}
