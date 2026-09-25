//! Pure word-wrap for the input box.
//!
//! The input box renders prose that frequently exceeds the terminal width. We
//! pre-split each logical line into visual lines with a greedy word wrap
//! (char count, see the limitation note below) and render a `Paragraph`
//! **without** ratatui's `.wrap()` — so this module is the single source of
//! truth for the rendered line breaks. The cursor mapping in [`super::cursor`]
//! reuses these same breaks, so the cursor can never disagree with what is
//! rendered — the trap the spec calls out (a wrap that does not match its own
//! cursor is worse than today's truncation).
//!
//! **Known limitation — display width.** Wrapping is by *char count*, not
//! terminal cell width (a CJK character counts as 1 here but occupies 2
//! cells). It no longer matches the transcript pane: `wrap_word_aware` went
//! cell-aware (the shared `tui::wrap` helper it and the approval panel both
//! use). The char model stays here because the cursor mapping in
//! [`super::cursor`] depends on these exact breaks; a long CJK line can
//! overflow the visible box and the cursor drifts right of its true cell.
//! Display-width-aware wrapping is the follow-up.

/// Greedy word wrap of one logical line into visual lines of at most `width`
/// chars. Fits as many words per line as `width` allows; the whitespace that
/// forces a break is dropped (not carried to the next line); leading whitespace
/// at the start of a line is kept; over-long single words hard-break at
/// `width`. An empty input yields one empty visual line.
pub(crate) fn wrap_line(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out: Vec<String> = Vec::new();
    let mut line: Vec<char> = Vec::new();
    let mut i = 0;
    while i < n {
        if chars[i].is_whitespace() {
            let start = i;
            while i < n && chars[i].is_whitespace() {
                i += 1;
            }
            let ws = &chars[start..i];
            if line.len() + ws.len() <= width {
                line.extend_from_slice(ws);
            } else {
                // The whitespace straddles the boundary: keep what fits on the
                // current line, drop the rest (the inter-word separator that
                // forced the break — trim:false keeps *leading* indentation,
                // not mid-word separators).
                let fits = width.saturating_sub(line.len());
                if fits > 0 {
                    line.extend_from_slice(&ws[..fits.min(ws.len())]);
                }
                out.push(take_line(&mut line));
            }
            continue;
        }
        let start = i;
        while i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        let word = &chars[start..i];
        if word.len() > width {
            hard_break(word, width, &mut out, &mut line);
            continue;
        }
        if line.len() + word.len() <= width {
            line.extend_from_slice(word);
        } else {
            out.push(take_line(&mut line));
            line.extend_from_slice(word);
        }
    }
    out.push(take_line(&mut line));
    out
}

/// Splits an over-long word into `width`-sized chunks, attaching the first
/// chunk to the current line when it fits and flushing otherwise.
fn hard_break(word: &[char], width: usize, out: &mut Vec<String>, line: &mut Vec<char>) {
    let mut k = 0;
    while k < word.len() {
        let take = width.min(word.len() - k);
        if line.len() + take <= width {
            line.extend_from_slice(&word[k..k + take]);
        } else {
            out.push(take_line(line));
            line.extend_from_slice(&word[k..k + take]);
        }
        k += take;
    }
}

/// Emits the current line, dropping trailing whitespace so wrapped lines
/// don't carry invisible spaces past their content.
fn take_line(line: &mut Vec<char>) -> String {
    while line.last().is_some_and(|c| c.is_whitespace()) {
        line.pop();
    }
    line.drain(..).collect()
}

/// Number of visual rows `text` occupies when wrapped to `width`, counting
/// each logical line's wrapped rows. Always at least 1.
pub(crate) fn visual_row_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    text.split('\n')
        .map(|line| wrap_line(line, width).len())
        .sum::<usize>()
        .max(1)
}

#[cfg(test)]
mod wrap_tests;
