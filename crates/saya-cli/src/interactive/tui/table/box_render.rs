use super::shared::{Alignment, format_cell};

/// Renders headers + string rows as an aligned box-drawing table (the lines between
/// the top border and the bottom border, inclusive - NO footer). `numeric[i] == true`
/// right-aligns column i's data cells; headers are always left-aligned. Column widths
/// are capped at 40 with ellipsis truncation, matching the existing behaviour.
pub(crate) fn render_box(
    headers: &[String],
    rows: &[Vec<String>],
    numeric: &[bool],
) -> Vec<String> {
    let num_cols = headers.len();
    if num_cols == 0 {
        return Vec::new();
    }

    let mut col_widths = Vec::with_capacity(num_cols);
    for (i, col_name) in headers.iter().enumerate() {
        let header_len = col_name.chars().count();
        let max_cell_len = rows
            .iter()
            .map(|r| r.get(i).map_or(0, |cell| cell.chars().count()))
            .max()
            .unwrap_or(0);
        let w = header_len.max(max_cell_len).min(40);
        col_widths.push(w);
    }

    let mut lines = Vec::with_capacity(rows.len() + 4);

    let mut top = String::from("┌");
    for (i, &w) in col_widths.iter().enumerate() {
        top.push_str(&"─".repeat(w + 2));
        if i + 1 < num_cols {
            top.push('┬');
        } else {
            top.push('┐');
        }
    }
    lines.push(top);

    let mut hdr = String::from("│");
    for (i, &w) in col_widths.iter().enumerate() {
        hdr.push(' ');
        hdr.push_str(&format_cell(&headers[i], w, Alignment::Left));
        hdr.push(' ');
        hdr.push('│');
    }
    lines.push(hdr);

    let mut sep = String::from("├");
    for (i, &w) in col_widths.iter().enumerate() {
        sep.push_str(&"─".repeat(w + 2));
        if i + 1 < num_cols {
            sep.push('┼');
        } else {
            sep.push('┤');
        }
    }
    lines.push(sep);

    for row in rows {
        let mut line = String::from("│");
        for (i, &w) in col_widths.iter().enumerate() {
            let align = if numeric.get(i).copied().unwrap_or(false) {
                Alignment::Right
            } else {
                Alignment::Left
            };
            let cell_str = row.get(i).map(|s| s.as_str()).unwrap_or("");
            line.push(' ');
            line.push_str(&format_cell(cell_str, w, align));
            line.push(' ');
            line.push('│');
        }
        lines.push(line);
    }

    let mut bot = String::from("└");
    for (i, &w) in col_widths.iter().enumerate() {
        bot.push_str(&"─".repeat(w + 2));
        if i + 1 < num_cols {
            bot.push('┴');
        } else {
            bot.push('┘');
        }
    }
    lines.push(bot);

    lines
}
