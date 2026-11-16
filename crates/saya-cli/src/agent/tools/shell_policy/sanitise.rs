//! Sanitising the model-authored strings an operator's prompt paints.

/// What a control or invisible format character is shown as.
const REPLACEMENT: char = '\u{FFFD}';

/// One model-authored string, safe to paint on an operator's screen: every
/// control character except `\n` and `\t` — ESC and `\r` included — and every
/// invisible format character becomes [`REPLACEMENT`]. Nothing is cut or
/// rewritten: the command stays literal, a newline in it a newline exactly as
/// the child will receive it, so what the operator reads is what runs. A
/// surface that must not show an unbounded command bounds it itself — the
/// coding panel refuses one longer than its prompt; the session's facts paint
/// a bounded head and tail and name the private file holding the withheld
/// bytes, because the schema admits a 64 KiB command.
pub(crate) fn sanitise_prompt_text(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() || is_invisible_format(c) => REPLACEMENT,
            c => c,
        })
        .collect()
}

/// The Unicode Format (`Cf`) characters that paint as nothing while they still
/// steer what the operator reads: the bidi marks, embeddings and overrides, the
/// bidi isolates, the zero-width marks and joiners, and the byte-order mark.
/// Listed as explicit ranges — the standard library exposes no general-category
/// table, and a prompt must not grow a dependency to get one.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}
