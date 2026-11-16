//! Coarse human ages for the record block's fact timestamps: the only time
//! math the block does, injectable through the builder's `now` parameter.

/// A coarse human age from an observed timestamp against a clock reading:
/// sub-minute, whole minutes, hours, or days — and `just now` for a
/// timestamp in the future of that clock.
pub(super) fn age_text(observed_unix_ms: i64, now_unix_ms: i64) -> String {
    let delta = now_unix_ms - observed_unix_ms;
    if delta < 0 {
        "just now".to_owned()
    } else if delta < 60_000 {
        "<1m".to_owned()
    } else if delta < 3_600_000 {
        format!("{}m", delta / 60_000)
    } else if delta < 86_400_000 {
        format!("{}h", delta / 3_600_000)
    } else {
        format!("{}d", delta / 86_400_000)
    }
}

/// The rendered parenthetical a fact line ends with: past ages read
/// `(observed Nm ago)`; a future timestamp reads `(observed just now)`.
pub(super) fn age_suffix(observed_unix_ms: i64, now_unix_ms: i64) -> String {
    match age_text(observed_unix_ms, now_unix_ms).as_str() {
        "just now" => "(observed just now)".to_owned(),
        age => format!("(observed {age} ago)"),
    }
}
