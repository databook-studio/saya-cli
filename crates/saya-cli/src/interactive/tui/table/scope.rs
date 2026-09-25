//! The scope line under a result table: the connection the result came from
//! and the query that produced it, frozen at completion time.
//!
//! Format, with `…` marking a truncated query (chars, matching the box
//! renderer): `from <profile> · <sql>` when a connection was captured at
//! dispatch, `<sql>` alone when none was. Only two values already in hand
//! feed it: `Followup::Sql { connection }` (the dispatch-time profile) and
//! `QueryResult.executed_sql`. No period, units, filters, or timing — none
//! exists anywhere, and none is invented here. The line claims no total:
//! it appends to `format_table` output (which already carries packet 1's
//! floor footer), never rewrites it.

/// Query chars kept on the scope line before an ellipsis marks the cut.
/// Long SQL stays whole in `executed_sql` and `block.text`; only this
/// painted line is shortened.
pub(crate) const SCOPE_SQL_CHARS: usize = 80;

/// Renders the scope line from values already in hand: the dispatch-time
/// connection (never the live profile — that is the gate) and the executed
/// SQL, collapsed to one line and truncated with an ellipsis past
/// [`SCOPE_SQL_CHARS`] chars. `None` when there is nothing honest to say
/// (no connection and no query); the caller then appends nothing.
pub(crate) fn scope_line(connection: Option<&str>, executed_sql: &str) -> Option<String> {
    let query = single_line(executed_sql);
    let short = truncate_chars(&query, SCOPE_SQL_CHARS);
    match connection.filter(|c| !c.is_empty()) {
        Some(profile) if short.is_empty() => Some(format!("from {profile}")),
        Some(profile) => Some(format!("from {profile} · {short}")),
        None if short.is_empty() => None,
        None => Some(short),
    }
}

/// Appends the scope line (when [`scope_line`] yields one) beneath
/// `format_table` output, after the row-count footer. The footer stays the
/// last count line; the scope is provenance, not a denominator.
pub(crate) fn with_scope_line(
    table_text: String,
    connection: Option<&str>,
    executed_sql: &str,
) -> String {
    match scope_line(connection, executed_sql) {
        Some(line) => format!("{table_text}\n{line}"),
        None => table_text,
    }
}

/// Collapses the SQL to one display line: newlines become spaces, kept
/// verbatim otherwise.
fn single_line(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Keeps the first `limit` chars, marking the cut with `…` (chars, like the
/// box renderer's cell cap and the chapter fold's width).
fn truncate_chars(text: &str, limit: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        return text.to_string();
    }
    let mut short: String = chars[..limit.saturating_sub(1)].iter().collect();
    short.push('…');
    short
}
