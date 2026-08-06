/// Formats a SQL string for readable multi-line display. Purely cosmetic and
/// NEVER used to build a query that executes: it collapses runs of whitespace to
/// single spaces, then inserts a line break before each major clause keyword
/// (case-insensitive, whole word), preserving the original casing of the text.
pub(crate) fn format_sql(sql: &str) -> String {
    // Multi-word keywords must be checked before their single-word prefixes.
    const KEYWORDS: &[&str] = &[
        "LEFT JOIN",
        "RIGHT JOIN",
        "INNER JOIN",
        "OUTER JOIN",
        "FULL JOIN",
        "CROSS JOIN",
        "GROUP BY",
        "ORDER BY",
        "UNION ALL",
        "FROM",
        "WHERE",
        "HAVING",
        "LIMIT",
        "OFFSET",
        "JOIN",
        "UNION",
        "VALUES",
        "RETURNING",
    ];
    // 1) collapse whitespace
    let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    // 2) walk tokens by char; before a keyword match at a word boundary
    //    (not at position 0), start a new line.
    let bytes = collapsed.as_bytes();
    let mut out = String::with_capacity(collapsed.len() + 16);
    let mut i = 0usize;
    while i < collapsed.len() {
        // Only consider a break at a word start: i == 0 handled (no break), or prev char is space.
        let at_word_start = i == 0 || bytes[i - 1] == b' ';
        let mut matched: Option<usize> = None; // length of matched keyword
        if at_word_start && i != 0 {
            for kw in KEYWORDS {
                let end = i + kw.len();
                if end <= collapsed.len()
                    && collapsed.is_char_boundary(end)
                    && collapsed[i..end].eq_ignore_ascii_case(kw)
                    && (end == collapsed.len() || bytes[end] == b' ')
                {
                    matched = Some(kw.len());
                    break;
                }
            }
        }
        if matched.is_some() {
            // Trim the single space we already emitted before this keyword, then newline.
            if out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
        }
        let ch = collapsed[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Collapses runs of whitespace (including newlines) into single spaces so
/// multi-line model SQL renders as one tidy line across every surface.
pub(super) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
