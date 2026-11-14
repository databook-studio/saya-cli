//! Bounded, line-numbered excerpts of what a write or edit changed: enough
//! of the changed region for the model to see the result, never the file.
//!
//! Lines are numbered as the read tool numbers them (`   123→text`), so an
//! excerpt is read exactly like a read's `content`. A changed region carries
//! a few lines of context; a hunk is capped at 40 lines / 4 KiB, the whole
//! result at 12 KiB, and every cut is marked `... (cut)`. A write's head
//! excerpt is shorter (20 lines / 2 KiB) because the model wrote the bytes.

use std::ops::Range;

/// Lines of context on each side of a changed line.
pub(super) const EXCERPT_CONTEXT_LINES: usize = 3;
/// Most numbered lines one changed hunk carries.
pub(super) const EXCERPT_MAX_HUNK_LINES: usize = 40;
/// Most bytes one changed hunk carries, marker included.
pub(super) const EXCERPT_MAX_HUNK_BYTES: usize = 4 * 1024;
/// Most bytes the whole excerpt carries, markers included.
pub(super) const EXCERPT_MAX_BYTES: usize = 12 * 1024;
/// Most numbered lines a write's head excerpt carries.
pub(super) const HEAD_EXCERPT_MAX_LINES: usize = 20;
/// Most bytes a write's head excerpt carries, marker included.
pub(super) const HEAD_EXCERPT_MAX_BYTES: usize = 2 * 1024;

/// The marker that says the excerpt stopped before the change did.
pub(super) const CUT_MARKER: &str = "... (cut)";
/// The marker between two hunks that are not adjacent.
const GAP_MARKER: &str = "...";

struct Line<'a> {
    number: u64,
    start: usize,
    end: usize,
    text: &'a str,
}

/// The changed region of `text`, numbered and bounded. `changed` are byte
/// ranges in `text` (empty ranges are insertion points). `None` when `text`
/// has no lines or no range touches it.
pub(super) fn changed_excerpt(text: &str, changed: &[Range<usize>]) -> Option<String> {
    let lines = line_spans(text);
    if lines.is_empty() {
        return None;
    }
    let mut windows: Vec<Range<usize>> = Vec::new();
    for range in changed {
        let Some((first, last)) = touched_lines(&lines, range) else {
            continue;
        };
        let start = first.saturating_sub(EXCERPT_CONTEXT_LINES);
        let end = (last + 1 + EXCERPT_CONTEXT_LINES).min(lines.len());
        windows.push(start..end);
    }
    windows.sort_by_key(|window| window.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for window in windows {
        match merged.last_mut() {
            Some(previous) if window.start <= previous.end => {
                previous.end = previous.end.max(window.end);
            }
            _ => merged.push(window),
        }
    }
    render_windows(&lines, &merged)
}

/// The first lines of `text`, numbered and bounded: what a write of a new
/// file shows instead of a diff. `None` when `text` is empty.
pub(super) fn head_excerpt(text: &str) -> Option<String> {
    let lines = line_spans(text);
    if lines.is_empty() {
        return None;
    }
    let window_end = lines.len().min(HEAD_EXCERPT_MAX_LINES);
    let (mut hunk, mut cut) = render_hunk(
        &lines,
        0..window_end,
        HEAD_EXCERPT_MAX_LINES,
        HEAD_EXCERPT_MAX_BYTES,
    );
    if lines.len() > window_end {
        cut = true;
    }
    if cut {
        push_cut_marker(&mut hunk, HEAD_EXCERPT_MAX_BYTES);
    }
    if hunk.is_empty() {
        None
    } else {
        Some(hunk.trim_end_matches('\n').to_owned())
    }
}

fn line_spans(text: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut number = 1u64;
    for (index, _) in text.match_indices('\n') {
        let end = index + 1;
        lines.push(Line {
            number,
            start,
            end,
            text: &text[start..end],
        });
        start = end;
        number += 1;
    }
    if start < text.len() {
        lines.push(Line {
            number,
            start,
            end: text.len(),
            text: &text[start..],
        });
    }
    lines
}

/// The first and last line a byte range touches. An empty range is an
/// insertion point: it touches the line it lands in (the last line at EOF).
fn touched_lines(lines: &[Line<'_>], range: &Range<usize>) -> Option<(usize, usize)> {
    let last_index = lines.len() - 1;
    if range.start == range.end {
        if range.start >= lines[last_index].end {
            return Some((last_index, last_index));
        }
        let first = lines
            .iter()
            .position(|line| line.end > range.start)
            .unwrap_or(last_index);
        return Some((first, first));
    }
    let first = lines
        .iter()
        .position(|line| line.end > range.start)
        .unwrap_or(last_index);
    let last = lines
        .iter()
        .rposition(|line| line.start < range.end)
        .unwrap_or(first)
        .max(first);
    Some((first, last))
}

fn render_windows(lines: &[Line<'_>], windows: &[Range<usize>]) -> Option<String> {
    if windows.is_empty() {
        return None;
    }
    let mut out = String::new();
    let mut cut = false;
    for window in windows {
        let (hunk, hunk_cut) = render_hunk(
            lines,
            window.clone(),
            EXCERPT_MAX_HUNK_LINES,
            EXCERPT_MAX_HUNK_BYTES,
        );
        let gap = if out.is_empty() {
            0
        } else if out.ends_with('\n') {
            GAP_MARKER.len() + 1
        } else {
            GAP_MARKER.len() + 2
        };
        if out.len() + gap + hunk.len() + CUT_MARKER.len() + 1 > EXCERPT_MAX_BYTES {
            cut = true;
            break;
        }
        if !out.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(GAP_MARKER);
            out.push('\n');
        }
        out.push_str(&hunk);
        if hunk_cut {
            cut = true;
            break;
        }
    }
    if cut {
        push_cut_marker(&mut out, EXCERPT_MAX_BYTES);
    }
    if out.is_empty() {
        None
    } else {
        Some(out.trim_end_matches('\n').to_owned())
    }
}

/// Renders one window of lines under the line and byte caps. The returned
/// flag says the hunk stopped before the window did.
fn render_hunk(
    lines: &[Line<'_>],
    window: Range<usize>,
    max_lines: usize,
    max_bytes: usize,
) -> (String, bool) {
    let budget = max_bytes.saturating_sub(CUT_MARKER.len() + 1);
    let mut out = String::new();
    let mut cut = false;
    for (count, line) in lines[window].iter().enumerate() {
        if count >= max_lines {
            cut = true;
            break;
        }
        let rendered = format!("{:>6}→{}", line.number, line.text);
        if out.len() + rendered.len() > budget {
            let remaining = budget.saturating_sub(out.len());
            if remaining > 0 {
                out.push_str(&truncate_line(line, remaining));
            }
            cut = true;
            break;
        }
        out.push_str(&rendered);
    }
    (out, cut)
}

/// A line cut to `budget` bytes with its number prefix intact, never inside
/// a UTF-8 character.
fn truncate_line(line: &Line<'_>, budget: usize) -> String {
    let prefix = format!("{:>6}→", line.number);
    if budget <= prefix.len() {
        return truncate_chars(&prefix, budget).to_owned();
    }
    let body = truncate_chars(line.text, budget - prefix.len());
    format!("{prefix}{body}")
}

fn truncate_chars(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn push_cut_marker(out: &mut String, max_bytes: usize) {
    let needed = CUT_MARKER.len() + usize::from(!out.ends_with('\n'));
    if out.len() + needed > max_bytes {
        return;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(CUT_MARKER);
    out.push('\n');
}
