//! What the loop answers a stream cut with: one bounded continuation that
//! spends the closed note below. A gateway time-cut does not size an output
//! ceiling. It is a clock, not a measured output window.

/// Consecutive cuts the loop will answer before ending the job. A cut is a
/// clock, not an output window: the same allowance as output truncation would
/// let three cuts spread over hours end a long job. Consecutive means the next
/// completed reply resets the count, so a healthy run can survive any number of
/// cuts separated by real work.
pub(super) const MAX_CONSECUTIVE_STREAM_CUTS: u32 = 3;

/// Re-instruction pushed as a user-role message when the provider cut the
/// stream after it had already delivered content.
///
/// The incomplete output is discarded, never replayed: a half-assembled
/// tool-argument JSON would invite the model to complete a write whose first
/// half it never issued, and reasoning never becomes request history. The note
/// is closed — no provider text, no partial output.
pub(super) fn continuation_note(elapsed_secs: u32) -> String {
    format!(
        "Your previous response was cut off by the request time limit after \
         {elapsed_secs}s. The incomplete part was discarded and is not in the conversation. \
         Do not re-plan from the start."
    )
}

/// Ceiling a gateway time-cut may impose on later requests. Always `None`.
///
/// A time-cut (`ProviderError::StreamCut`, ~300 s, `stream_terminator_missing`)
/// is a clock, including one that leaked visible text or tool arguments.
/// Fitting 85% of that scrap and clamping it up to 1,024 pinned the rest of
/// a flash run there. Output-token truncation does not come through here.
pub(super) fn sticky_ceiling_after_cut(
    visible_tokens: u32,
    configured: Option<u32>,
) -> Option<u32> {
    // Both inputs stay on the signature so a caller cannot fit them without
    // changing this owner. They are not a window.
    let _ = (visible_tokens, configured);
    None
}

#[cfg(test)]
#[path = "stream_cut_tests.rs"]
mod tests;
