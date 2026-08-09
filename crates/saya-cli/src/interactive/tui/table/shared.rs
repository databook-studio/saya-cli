#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Alignment {
    Left,
    Right,
}

pub(super) fn get_cell(row_val: &serde_json::Value, col_idx: usize) -> Option<&serde_json::Value> {
    match row_val {
        serde_json::Value::Array(vals) => vals.get(col_idx),
        other => {
            if col_idx == 0 {
                Some(other)
            } else {
                None
            }
        }
    }
}

pub(super) fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "NULL".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn format_cell(text: &str, col_width: usize, alignment: Alignment) -> String {
    if col_width == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    let content = if char_count > col_width {
        let take_len = col_width.saturating_sub(1);
        let mut s: String = text.chars().take(take_len).collect();
        s.push('…');
        s
    } else {
        text.to_string()
    };
    let content_len = content.chars().count();
    let padding = " ".repeat(col_width.saturating_sub(content_len));
    match alignment {
        Alignment::Left => format!("{content}{padding}"),
        Alignment::Right => format!("{padding}{content}"),
    }
}

pub(super) fn clean_cell(cell: &str) -> String {
    let mut s = cell.trim();
    if s.starts_with("**") && s.ends_with("**") && s.len() >= 4 {
        s = &s[2..s.len() - 2];
        s = s.trim();
    }
    if s.starts_with('`') && s.ends_with('`') && s.len() >= 2 {
        s = &s[1..s.len() - 1];
        s = s.trim();
    }
    if s.starts_with("**") && s.ends_with("**") && s.len() >= 4 {
        s = &s[2..s.len() - 2];
        s = s.trim();
    }
    s.to_string()
}
