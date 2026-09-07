//! Cursor-to-visual-position mapping for the wrapped input box.
//!
//! [`cursor_visual`] maps a flat char-index cursor to a visual `(row, col)`
//! against a wrap of `width` columns, using the same line breaks as
//! [`super::wrap::wrap_line`] so the cursor can never disagree with what is
//! rendered. See the display-width limitation note on [`super::wrap`].

/// Visual `(row, col)` of the cursor at char index `cursor` within `text`,
/// wrapped to `width`. `cursor` is in `[0, char_count]`. Logical newlines
/// (`\n`) start a new visual row at column 0.
pub(crate) fn cursor_visual(text: &str, width: usize, cursor: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }
    // Walk logical lines, accumulating the char offset of each line's start.
    let mut line_start = 0usize; // char index where the current logical line begins
    let mut row = 0usize;
    for logical in text.split('\n') {
        let line_len = logical.chars().count();
        let line_end = line_start + line_len; // exclusive; cursor here is at end of this logical line
        if cursor <= line_end {
            let col = cursor - line_start;
            let (r, c) = cursor_visual_in_line(logical, width, col);
            return (row + r, c);
        }
        row += super::wrap::wrap_line(logical, width).len();
        line_start = line_end + 1; // skip the '\n'
    }
    // Cursor at the very end of the whole text (after a trailing newline) —
    // lands at the start of a fresh final line.
    (row, 0)
}

/// Visual (row, col) within a single logical line for a cursor at char `col`.
fn cursor_visual_in_line(text: &str, width: usize, col: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut row = 0usize;
    let mut line_len = 0usize; // committed chars on current visual row
    let mut i = 0usize;
    let target = col;
    while i < n {
        if chars[i].is_whitespace() {
            let start = i;
            while i < n && chars[i].is_whitespace() {
                i += 1;
            }
            let ws = i - start;
            if line_len + ws <= width {
                if target < i {
                    return (row, line_len + (target - start));
                }
                line_len += ws;
            } else {
                let fits = width.saturating_sub(line_len);
                let kept = fits.min(ws);
                if target < start + kept {
                    return (row, line_len + (target - start));
                }
                if target < i {
                    return (row, line_len + kept);
                }
                row += 1;
                line_len = 0;
            }
            continue;
        }
        let start = i;
        while i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        let word_len = i - start;
        if word_len > width {
            let mut k = 0;
            while k < word_len {
                let take = width.min(word_len - k);
                if line_len + take <= width {
                    if target < start + k + take {
                        return (row, line_len + (target - (start + k)));
                    }
                    line_len += take;
                } else {
                    row += 1;
                    line_len = 0;
                    if target < start + k + take {
                        return (row, target - (start + k));
                    }
                    line_len += take;
                }
                k += take;
            }
            continue;
        }
        if line_len + word_len <= width {
            if target < i {
                return (row, line_len + (target - start));
            }
            line_len += word_len;
        } else {
            row += 1;
            line_len = 0;
            if target < i {
                return (row, target - start);
            }
            line_len += word_len;
        }
    }
    (row, line_len)
}
