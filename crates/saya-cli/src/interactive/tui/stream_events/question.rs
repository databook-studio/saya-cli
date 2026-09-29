//! The clarification turn in the transcript (B3c): a `ClarificationNeeded`
//! event pushes a distinct Question block — the model asked instead of
//! assuming, and the user's next message answers it as ordinary input.

use super::{BlockKind, Transcript};
use crate::render::clarification_text;

/// Pushes the question block for one landed ask. The block is a `System`
/// block (the transcript's non-conversational kind) whose text is the shared
/// shaper's — the exact words the piped surface prints, with the options
/// numbered. `Transcript` stores line text without the trailing delimiter,
/// so the shaper's trailing newline is trimmed.
pub(crate) fn push_question(transcript: &mut Transcript, question: &str, options: &[String]) {
    let text = clarification_text(question, options);
    transcript.push(BlockKind::System, text.trim_end_matches('\n'));
}
