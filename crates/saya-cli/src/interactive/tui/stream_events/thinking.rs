//! Reasoning gating: the model's chain-of-thought reaches the transcript
//! only when the user opted in, as a dimmed `Thinking` block separate from
//! the assistant answer.

use super::{BlockKind, Transcript};

// The model's chain-of-thought. Shown only when the user opted in; otherwise
// accepted and dropped, so the event never reaches the catch-all and never
// renders as an error. When shown it lands as a dimmed `Thinking` block,
// separate from the assistant answer and visually subordinate to it. Reasoning
// is in-memory only: the transcript is never serialized, and the persisted
// session types carry role + content only, so holding it here cannot reach a
// session file regardless of the display toggle.
pub(crate) fn push_reasoning_text(transcript: &mut Transcript, text: &str, show_thinking: bool) {
    if show_thinking && !text.is_empty() {
        transcript.push(BlockKind::Thinking, text);
    }
}
