//! The one shape a `shell` call's detail rides: the command, secret-stripped and
//! bounded to a single line.
//!
//! A pipe reader — a CI log, a lane audit — sees this line *before* the call
//! runs, so it must say what will run without handing a credential to the log.
//! The strip is `saya_types::redact`, the pass the job journal already puts a
//! tool's arguments through (`commands::coding::transcript`) and the base pass
//! the harness scrubs a capture with: one stripper, not a second one here.
//! The bounds are the schema's own: 64 KiB can be stated, and the line carries
//! at most [`MAX_DETAIL_BYTES`] of it, cut on a character boundary and marked
//! with [`CUT_MARKER`] so the reader knows the command continues.

/// The most command bytes the detail carries. A schema-legal command may be
/// 64 KiB; a streamed line must not be.
const MAX_DETAIL_BYTES: usize = 512;

/// The visible marker a newline becomes: one JSON line, one line for a reader.
const NEWLINE_MARKER: char = '⏎';

/// The visible marker appended when the command was cut.
const CUT_MARKER: char = '…';

/// The detail for a `shell` call's `command`, or `None` when it states none.
///
/// Order matters. The strip runs first so a credential is replaced by the
/// marker before anything is cut, then newlines are flattened (the marker is
/// three bytes, so the bound must see the flattened text), then the bound. A
/// cut can therefore land inside a marker, never inside a credential: the
/// value is already gone by the time any byte is dropped.
pub(crate) fn shell_command_detail(command: &str) -> Option<String> {
    if command.is_empty() {
        return None;
    }
    let flattened = saya_types::redact(command)
        .chars()
        .map(|ch| if ch == '\n' { NEWLINE_MARKER } else { ch })
        .collect::<String>();
    if flattened.len() <= MAX_DETAIL_BYTES {
        return Some(flattened);
    }
    let mut end = MAX_DETAIL_BYTES;
    while end > 0 && !flattened.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!("{}{CUT_MARKER}", &flattened[..end]))
}
