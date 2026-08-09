#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Normal,
    SingleQuoted,
    DoubleQuoted,
    BacktickQuoted,
    DollarQuoted,
    LineComment,
    BlockComment,
}

/// Formats a SQL string for readable multi-line display. Purely cosmetic and
/// NEVER used to build a query that executes: it collapses runs of whitespace to
/// single spaces, then inserts a line break before each major clause keyword
/// (case-insensitive, whole word), preserving the original casing of the text.
/// Quoted values, identifiers, and comments are left intact.
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

    let collapsed = collapse_whitespace(sql);
    let bytes = collapsed.as_bytes();
    let mut out = String::with_capacity(collapsed.len() + 16);
    let mut state = ScanState::Normal;
    let mut dollar_tag = None;
    let mut i = 0usize;

    while i < collapsed.len() {
        let ch = collapsed[i..].chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();

        if state == ScanState::Normal && keyword_at(&collapsed, bytes, i, KEYWORDS) {
            while out.ends_with(' ') {
                out.pop();
            }
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }

        out.push(ch);
        match state {
            ScanState::Normal => match ch {
                '\'' => state = ScanState::SingleQuoted,
                '"' => state = ScanState::DoubleQuoted,
                '`' => state = ScanState::BacktickQuoted,
                '$' if dollar_quote_tag_at(&collapsed, i).is_some() => {
                    let tag = dollar_quote_tag_at(&collapsed, i).expect("checked above");
                    out.push_str(&tag[1..]);
                    i += tag.len() - 1;
                    dollar_tag = Some(tag.to_string());
                    state = ScanState::DollarQuoted;
                }
                '-' if bytes.get(i + 1) == Some(&b'-') => state = ScanState::LineComment,
                '/' if bytes.get(i + 1) == Some(&b'*') => state = ScanState::BlockComment,
                _ => {}
            },
            ScanState::SingleQuoted if ch == '\'' => {
                if bytes.get(i + 1) == Some(&b'\'') {
                    out.push('\'');
                    i += 1;
                } else {
                    state = ScanState::Normal;
                }
            }
            ScanState::DoubleQuoted if ch == '"' => {
                if bytes.get(i + 1) == Some(&b'"') {
                    out.push('"');
                    i += 1;
                } else {
                    state = ScanState::Normal;
                }
            }
            ScanState::BacktickQuoted if ch == '`' => {
                if bytes.get(i + 1) == Some(&b'`') {
                    out.push('`');
                    i += 1;
                } else {
                    state = ScanState::Normal;
                }
            }
            ScanState::DollarQuoted
                if dollar_tag
                    .as_ref()
                    .is_some_and(|tag| collapsed[i..].starts_with(tag)) =>
            {
                let tag = dollar_tag.take().expect("dollar-quoted state has a tag");
                out.push_str(&tag[1..]);
                i += tag.len() - 1;
                state = ScanState::Normal;
            }
            ScanState::LineComment if ch == '\n' => state = ScanState::Normal,
            ScanState::BlockComment if ch == '/' && i > 0 && bytes[i - 1] == b'*' => {
                state = ScanState::Normal;
            }
            _ => {}
        }
        i += ch_len;
    }
    out
}

fn keyword_at(text: &str, bytes: &[u8], start: usize, keywords: &[&str]) -> bool {
    if start == 0 || !bytes[start - 1].is_ascii_whitespace() {
        return false;
    }
    keywords.iter().any(|keyword| {
        let end = start + keyword.len();
        end <= text.len()
            && text.is_char_boundary(end)
            && text[start..end].eq_ignore_ascii_case(keyword)
            && (end == text.len() || bytes[end].is_ascii_whitespace())
    })
}

fn dollar_quote_tag_at(text: &str, start: usize) -> Option<&str> {
    let remainder = text.get(start..)?;
    let end = remainder[1..].find('$')? + 1;
    let tag = &remainder[..=end];
    let name = &tag[1..tag.len() - 1];
    if name.is_empty() {
        return Some(tag);
    }
    let first = name.chars().next()?;
    (first.is_ascii_alphabetic() || first == '_').then_some(())?;
    name.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        .then_some(tag)
}

/// Collapses runs of whitespace outside quoted values and comments into single
/// spaces, so model SQL renders tidily without misrepresenting literal text.
pub(super) fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut state = ScanState::Normal;
    let mut dollar_tag = None;
    let mut pending_space = false;
    let mut i = 0usize;
    let bytes = text.as_bytes();

    while i < text.len() {
        let ch = text[i..].chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();

        match state {
            ScanState::Normal if ch.is_whitespace() => pending_space = !out.is_empty(),
            ScanState::Normal => {
                if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                out.push(ch);
                match ch {
                    '\'' => state = ScanState::SingleQuoted,
                    '"' => state = ScanState::DoubleQuoted,
                    '`' => state = ScanState::BacktickQuoted,
                    '$' if dollar_quote_tag_at(text, i).is_some() => {
                        let tag = dollar_quote_tag_at(text, i).expect("checked above");
                        out.push_str(&tag[1..]);
                        i += tag.len() - 1;
                        dollar_tag = Some(tag.to_string());
                        state = ScanState::DollarQuoted;
                    }
                    '-' if bytes.get(i + 1) == Some(&b'-') => state = ScanState::LineComment,
                    '/' if bytes.get(i + 1) == Some(&b'*') => state = ScanState::BlockComment,
                    _ => {}
                }
            }
            ScanState::SingleQuoted if ch == '\'' => {
                out.push(ch);
                if bytes.get(i + 1) == Some(&b'\'') {
                    out.push('\'');
                    i += 1;
                } else {
                    state = ScanState::Normal;
                }
            }
            ScanState::DoubleQuoted if ch == '"' => {
                out.push(ch);
                if bytes.get(i + 1) == Some(&b'"') {
                    out.push('"');
                    i += 1;
                } else {
                    state = ScanState::Normal;
                }
            }
            ScanState::BacktickQuoted if ch == '`' => {
                out.push(ch);
                if bytes.get(i + 1) == Some(&b'`') {
                    out.push('`');
                    i += 1;
                } else {
                    state = ScanState::Normal;
                }
            }
            ScanState::DollarQuoted
                if dollar_tag
                    .as_ref()
                    .is_some_and(|tag| text[i..].starts_with(tag)) =>
            {
                let tag = dollar_tag.take().expect("dollar-quoted state has a tag");
                out.push_str(&tag);
                i += tag.len() - ch_len;
                state = ScanState::Normal;
            }
            ScanState::LineComment if ch == '\n' => {
                out.push(ch);
                state = ScanState::Normal;
            }
            ScanState::BlockComment if ch == '/' && i > 0 && bytes[i - 1] == b'*' => {
                out.push(ch);
                state = ScanState::Normal;
            }
            _ => out.push(ch),
        }
        i += ch_len;
    }

    out
}
