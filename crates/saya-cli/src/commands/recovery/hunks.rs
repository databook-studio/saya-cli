//! Hunk emission for the bounded restore preview: two context lines
//! around every change, `@@` headers, GNU-style no-newline markers, and a
//! hard output cap with a truncation note — never more output than the
//! bounds allow, whatever the input shape.

/// Emitted diff body lines before truncation.
pub(crate) const DIFF_MAX_OUTPUT_LINES: usize = 200;

use super::line_diff::Op;

/// Renders the ops as unified-style hunks. Change ops keep two context
/// lines on each side, and runs whose context windows touch merge into
/// one hunk. Returns the text and whether any body line was cut.
pub(crate) fn emit(
    path: &str,
    before: &[&str],
    current: &[&str],
    ops: &[Op],
    before_trailing: bool,
    current_trailing: bool,
) -> (String, bool) {
    let mut marked = vec![false; ops.len()];
    for (index, op) in ops.iter().enumerate() {
        if matches!(op, Op::Same(_)) {
            continue;
        }
        let start = index.saturating_sub(2);
        let end = (index + 2).min(ops.len() - 1);
        for slot in marked[start..=end].iter_mut() {
            *slot = true;
        }
    }
    let mut lines = vec![
        format!("diff preview: {path}"),
        "--- before (recorded)".to_owned(),
        "+++ current".to_owned(),
    ];
    let mut body = 0usize;
    let mut truncated = false;
    let mut more = 0usize;
    let (mut a_next, mut b_next) = (0usize, 0usize);
    let mut index = 0usize;
    while index < ops.len() {
        if !marked[index] {
            advance(&ops[index], &mut a_next, &mut b_next);
            index += 1;
            continue;
        }
        let start = index;
        while index < ops.len() && marked[index] {
            index += 1;
        }
        let hunk = &ops[start..index];
        let a_count = hunk.iter().filter(|op| !matches!(op, Op::Ins(_))).count();
        let b_count = hunk.iter().filter(|op| !matches!(op, Op::Del(_))).count();
        let a_first = if a_count > 0 { a_next + 1 } else { a_next };
        let b_first = if b_count > 0 { b_next + 1 } else { b_next };
        push_body(
            &mut lines,
            &mut body,
            &mut truncated,
            &mut more,
            format!("@@ -{a_first},{a_count} +{b_first},{b_count} @@"),
        );
        for op in hunk {
            match op {
                Op::Same(i) => {
                    a_next += 1;
                    b_next += 1;
                    push_body(&mut lines, &mut body, &mut truncated, &mut more, {
                        format!(" {}", before[*i])
                    });
                }
                Op::Del(i) => {
                    a_next += 1;
                    push_body(&mut lines, &mut body, &mut truncated, &mut more, {
                        format!("-{}", before[*i])
                    });
                    if *i + 1 == before.len() && !before_trailing {
                        push_body(
                            &mut lines,
                            &mut body,
                            &mut truncated,
                            &mut more,
                            "\\ No newline at end of file".to_owned(),
                        );
                    }
                }
                Op::Ins(j) => {
                    b_next += 1;
                    push_body(&mut lines, &mut body, &mut truncated, &mut more, {
                        format!("+{}", current[*j])
                    });
                    if *j + 1 == current.len() && !current_trailing {
                        push_body(
                            &mut lines,
                            &mut body,
                            &mut truncated,
                            &mut more,
                            "\\ No newline at end of file".to_owned(),
                        );
                    }
                }
            }
        }
    }
    if truncated {
        lines.push(format!("… diff truncated ({more} more lines)"));
    }
    (lines.join("\n"), truncated)
}

/// Steps one out-of-hunk op past the side positions it consumed.
fn advance(op: &Op, a_next: &mut usize, b_next: &mut usize) {
    match op {
        Op::Same(_) => {
            *a_next += 1;
            *b_next += 1;
        }
        Op::Del(_) => *a_next += 1,
        Op::Ins(_) => *b_next += 1,
    }
}

/// Emits one body line under the output cap; dropped lines are counted
/// and reported by the truncation note.
fn push_body(
    lines: &mut Vec<String>,
    body: &mut usize,
    truncated: &mut bool,
    more: &mut usize,
    line: String,
) {
    let emit = *body < DIFF_MAX_OUTPUT_LINES;
    *body += 1;
    if emit {
        lines.push(line);
    } else {
        *truncated = true;
        *more += 1;
    }
}
