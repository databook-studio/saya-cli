//! The in-house line diff core behind the bounded restore preview: UTF-8
//! line splitting and a classic suffix-LCS over lines computed row by
//! row. No dependency, no I/O, deterministic.

/// Per-side byte bound for a diff.
pub(crate) const DIFF_MAX_BYTES: usize = 64 * 1024;
/// Per-side line bound.
pub(crate) const DIFF_MAX_LINES: usize = 2000;
/// LCS table cell bound: the table is never larger than this.
pub(crate) const DIFF_MAX_CELLS: usize = 4_000_000;

/// A side is diffable only as UTF-8 text without a NUL byte.
pub(crate) fn text_side(bytes: &[u8]) -> Option<&str> {
    if bytes.contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

/// Splits text into lines without their newline separators, reporting
/// whether the text ends with a newline; an empty text has no lines.
pub(crate) fn split_lines(text: &str) -> (Vec<&str>, bool) {
    if text.is_empty() {
        return (Vec::new(), false);
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    let trailing = lines.last() == Some(&"");
    if trailing {
        lines.pop();
    }
    (lines, trailing)
}

/// One diff step over the two line sequences.
pub(crate) enum Op {
    Same(usize),
    Del(usize),
    Ins(usize),
}

/// The classic suffix-LCS over lines, computed row by row into a table
/// already bounded by the cell check, then traced forward into ops.
pub(crate) fn lcs_ops(before: &[&str], current: &[&str]) -> Vec<Op> {
    let n = before.len();
    let m = current.len();
    let stride = m + 1;
    let mut table = vec![0u32; (n + 1) * stride];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * stride + j] = if before[i] == current[j] {
                table[(i + 1) * stride + j + 1] + 1
            } else {
                table[(i + 1) * stride + j].max(table[i * stride + j + 1])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if before[i] == current[j] {
            ops.push(Op::Same(i));
            i += 1;
            j += 1;
        } else if table[(i + 1) * stride + j] >= table[i * stride + j + 1] {
            ops.push(Op::Del(i));
            i += 1;
        } else {
            ops.push(Op::Ins(j));
            j += 1;
        }
    }
    ops.extend((i..n).map(Op::Del));
    ops.extend((j..m).map(Op::Ins));
    ops
}
