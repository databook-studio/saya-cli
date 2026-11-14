use saya_agent::{ToolError, shape_tool_result};
use serde_json::Value;

use super::super::{WORKSPACE_READ_MAX_BYTES, WORKSPACE_READ_MAX_LINES};
use super::LineRange;

pub(super) fn unranged_result(
    path: &str,
    file: saya_harness::workspace::ReadFile,
    cap: usize,
) -> Result<(Value, String, u64), ToolError> {
    let digest = file.digest.clone();
    let size = file.size;
    let bytes = file.bytes.clone();
    // An un-ranged read is the window from line 1, so it is bounded by the
    // same 2,000 lines a ranged read selects.
    let (start, end, _, _) = line_span(
        &bytes,
        LineRange {
            start: 1,
            count: WORKSPACE_READ_MAX_LINES,
        },
    );
    let selected = &bytes[start..end];
    let mut low = 0usize;
    let mut high = selected.len().min(WORKSPACE_READ_MAX_BYTES as usize);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let result = unranged_json(path, &file, &bytes, selected, middle);
        if within_cap(&result, cap) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let result = unranged_json(path, &file, &bytes, selected, low);
    if !within_cap(&result, cap) {
        return Err(no_effect("workspace read result exceeds caller cap"));
    }
    Ok((result, digest, size))
}

fn unranged_json(
    path: &str,
    file: &saya_harness::workspace::ReadFile,
    bytes: &[u8],
    selected: &[u8],
    take: usize,
) -> Value {
    let mut result = serde_json::json!({
        "path": path,
        "size": file.size,
        "truncated": file.truncated || take < selected.len() || selected.len() < bytes.len(),
        "digest": file.digest,
        "content": numbered_lines(&selected[..take], 1),
    });
    // A whole-file read carries no `truncated`: absence is the default the
    // description states, so the flag is served only when content was cut.
    crate::agent::tools::omit_default_flag(&mut result, "truncated", false);
    result
}

pub(super) fn ranged_result(
    path: &str,
    file: saya_harness::workspace::ReadFile,
    range: LineRange,
    cap: usize,
) -> Result<(Value, String, u64), ToolError> {
    let (start, end, total_lines, lines_returned) = line_span(&file.bytes, range);
    let selected = &file.bytes[start..end];
    let digest = file.digest.clone();
    let size = file.size;
    let max_content = selected.len().min(WORKSPACE_READ_MAX_BYTES as usize);
    let projection = RangeProjection {
        path,
        file: &file,
        range,
        selected,
        start,
        end,
        total_lines,
        lines_returned,
    };
    let mut low = 0usize;
    let mut high = max_content;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let result = ranged_json(&projection, middle);
        if within_cap(&result, cap) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let result = ranged_json(&projection, low);
    if !within_cap(&result, cap) {
        return Err(no_effect("workspace read result exceeds caller cap"));
    }
    Ok((result, digest, size))
}

struct RangeProjection<'a> {
    path: &'a str,
    file: &'a saya_harness::workspace::ReadFile,
    range: LineRange,
    selected: &'a [u8],
    start: usize,
    end: usize,
    total_lines: u64,
    lines_returned: u64,
}

fn ranged_json(projection: &RangeProjection<'_>, take: usize) -> Value {
    let selected_complete = take == projection.selected.len();
    let delivered = &projection.selected[..take];
    let delivered_lines = if selected_complete {
        projection.lines_returned
    } else {
        delivered.iter().filter(|byte| **byte == b'\n').count() as u64
    };
    let mut result = serde_json::json!({
        "path": projection.path,
        "size": projection.file.size,
        "truncated": take != projection.file.bytes.len()
            || projection.start != 0
            || projection.end != projection.file.bytes.len(),
        "digest": projection.file.digest,
        "content": numbered_lines(delivered, projection.range.start),
        "start_line": projection.range.start,
        "line_count": projection.range.count,
        "total_lines": projection.total_lines,
        "lines_returned": delivered_lines,
        "selected_range_complete": selected_complete,
    });
    // Only a range that delivered the whole file is untruncated; then the
    // flag says nothing the other fields do not, so it is left out.
    crate::agent::tools::omit_default_flag(&mut result, "truncated", false);
    result
}

/// The delivered bytes with every line carrying its 1-based number:
/// `   123→text`, right-aligned in six columns and separated by `→`. This is
/// the whole of a text read's `content` — the bytes after each separator are
/// the file's own, so the text is sent once and the number prefix is never part
/// of the file. The digest an `edit` precondition states comes from the file,
/// not from this text.
fn numbered_lines(delivered: &[u8], first_line: u64) -> String {
    let text = String::from_utf8_lossy(delivered);
    let mut numbered = String::with_capacity(
        text.len() + 9 * delivered.iter().filter(|b| **b == b'\n').count() + 9,
    );
    let mut number = first_line;
    let mut rest = text.as_ref();
    while let Some(end) = rest.find('\n') {
        numbered.push_str(&format!("{number:>6}→{}", &rest[..=end]));
        number += 1;
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        numbered.push_str(&format!("{number:>6}→{rest}"));
    }
    numbered
}

fn line_span(bytes: &[u8], range: LineRange) -> (usize, usize, u64, u64) {
    let end_line = range.start + range.count - 1;
    let mut line = 0u64;
    let mut cursor = 0usize;
    let mut selected_start = None;
    let mut selected_end = 0usize;
    while cursor < bytes.len() {
        let end = bytes[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |offset| cursor + offset + 1);
        line += 1;
        if (range.start..=end_line).contains(&line) {
            selected_start.get_or_insert(cursor);
            selected_end = end;
        }
        cursor = end;
    }
    let start = selected_start.unwrap_or(bytes.len());
    (
        start,
        selected_end.max(start),
        line,
        line.saturating_sub(range.start - 1).min(range.count),
    )
}

fn within_cap(value: &Value, cap: usize) -> bool {
    !shape_tool_result(value, cap).truncated
}

fn no_effect(message: &'static str) -> ToolError {
    ToolError::NoEffect(Box::new(ToolError::Runner(message.into())))
}
