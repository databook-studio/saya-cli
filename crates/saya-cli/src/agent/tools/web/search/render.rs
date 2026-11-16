//! The answer as the model reads it: one untrusted block, bounded.
//!
//! Results are untrusted text — a title and a snippet are whatever a stranger
//! wrote, and a search engine is exactly how a stranger gets to write
//! something an agent will read. They reach the model the only way they may:
//! as a [`ContextBlock`], rendered by `saya_agent::render_untrusted_block`,
//! inside the same honest envelope a fetched page uses.
//!
//! The whole rendering is cut to [`MAX_RESULT_BYTES`] through the runner's
//! head-and-tail ring, so a backend answering with ten enormous results still
//! produces a bounded block whose elision marker sits inside the delimiters.

use saya_agent::ContextBlock;
use saya_harness::runner::OutputRing;

use super::bounds::MAX_RESULT_BYTES;
use super::parse::Hit;

/// What the model is told when the backend answered with nothing. Said
/// plainly, and in the block's own body, so "nothing matched" is never
/// mistaken for "the tool returned nothing".
const NO_RESULTS: &str = "no results";

/// One answer as the block the model reads, and the bytes the bound elided.
/// A result that is not a hit at all — an entry with no URL — never reaches
/// here; [`super::parse`] has already dropped it.
pub(crate) fn block(label: String, hits: &[Hit]) -> (ContextBlock, u64) {
    let rendered = render(hits);
    let ring = OutputRing::new(MAX_RESULT_BYTES);
    ring.push(rendered.as_bytes());
    let (body, elided, elided_bytes, _) = ring.snapshot();
    (
        ContextBlock {
            label,
            body,
            // Only the byte bound cutting the answer sets this: a title or a
            // snippet cut to its own per-field bound is already reported by
            // the field's own length, and the marker's sentence — "the source
            // had more than the budget allowed" — is a claim about the answer
            // as a whole.
            truncated: elided,
        },
        elided_bytes,
    )
}

/// The answer as text: one numbered result per hit, its URL under its title
/// and its snippet under that. Numbered so the model can refer to a result
/// without quoting a URL back, and laid out one field per line so a title
/// containing a newline cannot be read as a second result.
fn render(hits: &[Hit]) -> String {
    if hits.is_empty() {
        return NO_RESULTS.to_owned();
    }
    let mut rendered = String::new();
    for (index, hit) in hits.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&format!("{}. {}\n", index + 1, one_line(&hit.title)));
        rendered.push_str(&format!("   {}\n", one_line(&hit.url)));
        rendered.push_str(&format!("   {}\n", one_line(&hit.snippet)));
    }
    rendered
}

/// One field on one line: every control character — a newline above all —
/// becomes a space, so a field can never forge the layout of a second result.
/// The text is otherwise passed through unchanged; it is data, and the block's
/// renderer is what keeps it from being read as anything else.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}
