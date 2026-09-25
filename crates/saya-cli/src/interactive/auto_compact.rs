//! The automatic-compaction decision: the pure predicate the turn boundary
//! reads, and the automatic-path strings.
//!
//! The decision answers on the same numerator the warning and the footer use —
//! the last *answering* call's `input_tokens` over the known window — so the
//! three surfaces can never round differently. Unknown window (or no provider
//! report) means no decision, never zero: absence is not zero, and a stale
//! numerator from a previous turn must not fire either, so the caller passes
//! the turn's own report only.
//!
//! The trigger fires at or above `CONTEXT_COMPACT_PERCENT`, including past
//! 100%: a stale numerator means the true figure may already exceed the
//! window, and that is exactly when compaction is most needed.

use saya_agent::{CONTEXT_COMPACT_PERCENT, context_utilisation_percent};
use saya_config::CompactionMode;

/// Whether the automatic trigger may fire on this turn's report.
pub(crate) struct AutoCompactInput {
    pub(crate) mode: CompactionMode,
    pub(crate) answering_input: Option<u64>,
    pub(crate) window: Option<u64>,
    /// `true` once the provider caps a response mid-answer and the loop
    /// re-instructs: the partial is discarded and the resume anchors live in
    /// earlier tool results — compacting there risks the very lines the
    /// resume needs. The caller passes whether the finished turn continued.
    pub(crate) continued: bool,
    /// `true` while a compaction worker is already running: one compaction at
    /// a time, so a second crossing waits rather than piling on.
    pub(crate) compact_running: bool,
    /// `true` after an automatic compaction already tried and failed this
    /// session: a session that fails to summarise every turn forever would
    /// burn the budget it was trying to save. Re-armed by `/clear` and by any
    /// successful compaction.
    pub(crate) auto_failed: bool,
}

/// The pure decision: fire only in `auto` mode, at a finished turn, with a
/// known window, a provider report, utilisation at or above the compact
/// threshold, no compaction running, and no outstanding automatic failure.
pub(crate) fn should_auto_compact(input: &AutoCompactInput) -> bool {
    if input.mode != CompactionMode::Auto {
        return false;
    }
    if input.continued || input.compact_running || input.auto_failed {
        return false;
    }
    let Some(percent) = context_utilisation_percent(input.answering_input, input.window) else {
        return false;
    };
    percent >= CONTEXT_COMPACT_PERCENT
}

/// The transcript line for an automatic success: the manual success message
/// with its origin stated, so the user knows it was automatic rather than
/// something they typed.
pub(crate) fn auto_success_message(compacted_turns: usize, summary: &str) -> String {
    format!(
        "Automatic compaction: {}",
        super::session_compact::success_message(compacted_turns, summary)
    )
}

/// The transcript line for an automatic failure: the manual failure message
/// with its origin stated, plus the no-retry policy, so the user knows the
/// trigger will stay silent until `/compact` or `/clear` re-arms it.
pub(crate) fn auto_failure_message(reason: &str) -> String {
    format!(
        "Automatic compaction: {}. It will not retry on its own; run /compact to try again.",
        super::session_compact::failure_message(reason)
    )
}

/// Whether the context-window warning may fire under this mode: `off`
/// silences it along with the trigger; every other mode warns as today.
pub(crate) fn warning_enabled(mode: CompactionMode) -> bool {
    mode != CompactionMode::Off
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(answering_input: Option<u64>, window: Option<u64>) -> AutoCompactInput {
        AutoCompactInput {
            mode: CompactionMode::Auto,
            answering_input,
            window,
            continued: false,
            compact_running: false,
            auto_failed: false,
        }
    }

    #[test]
    fn crossing_95_fires() {
        assert!(should_auto_compact(&input(Some(121_600), Some(128_000))));
    }

    #[test]
    fn past_100_is_a_real_case_that_fires() {
        assert!(should_auto_compact(&input(Some(200_000), Some(128_000))));
    }

    #[test]
    fn below_95_does_not_fire() {
        assert!(!should_auto_compact(&input(Some(64_000), Some(128_000))));
    }

    #[test]
    fn unknown_window_never_fires() {
        assert!(!should_auto_compact(&input(Some(122_880), None)));
    }

    #[test]
    fn no_provider_report_never_fires_absent_is_not_zero() {
        assert!(!should_auto_compact(&input(None, Some(128_000))));
    }

    #[test]
    fn manual_and_off_never_fire() {
        for mode in [CompactionMode::Manual, CompactionMode::Off] {
            assert!(
                !should_auto_compact(&AutoCompactInput {
                    mode,
                    ..input(Some(122_880), Some(128_000))
                }),
                "{mode:?} must never fire automatically"
            );
        }
    }

    #[test]
    fn mid_continuation_never_fires() {
        assert!(
            !should_auto_compact(&AutoCompactInput {
                continued: true,
                ..input(Some(122_880), Some(128_000))
            }),
            "a continued turn must not compact: the resume anchors live in earlier tool results"
        );
    }

    #[test]
    fn a_running_compaction_and_a_failed_one_both_suppress() {
        assert!(!should_auto_compact(&AutoCompactInput {
            compact_running: true,
            ..input(Some(122_880), Some(128_000))
        }));
        assert!(!should_auto_compact(&AutoCompactInput {
            auto_failed: true,
            ..input(Some(122_880), Some(128_000))
        }));
    }

    #[test]
    fn the_automatic_strings_reuse_the_manual_path_with_origin_stated() {
        let manual = super::super::session_compact::success_message(3, "older turns at length");
        assert_eq!(
            auto_success_message(3, "older turns at length"),
            format!("Automatic compaction: {manual}")
        );
        let manual_fail =
            super::super::session_compact::failure_message("the summariser timed out");
        let auto = auto_failure_message("the summariser timed out");
        assert!(auto.starts_with("Automatic compaction: "));
        assert!(auto.contains(&manual_fail));
        assert!(auto.contains("/compact"));
    }

    #[test]
    fn off_silences_the_warning_while_other_modes_warn() {
        assert!(!warning_enabled(CompactionMode::Off));
        assert!(warning_enabled(CompactionMode::Auto));
        assert!(warning_enabled(CompactionMode::Manual));
    }
}
