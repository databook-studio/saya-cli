use super::box_render::render_box;
use super::shared::clean_cell;

/// Rewrites any GitHub-style Markdown pipe tables found in `text` into box-drawing tables
/// (the same style as query results), leaving all other lines untouched. A table is only
/// recognised when a header row `| a | b |` is immediately followed by a separator row whose
/// cells are all dashes/colons (e.g. `|---|:--:|`) - so stray `|` in prose is never mangled.
#[allow(dead_code)]
pub(crate) fn format_markdown_tables(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut output = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len() {
            let header_opt = parse_line_raw_cells(lines[i]);
            let sep_opt = parse_line_raw_cells(lines[i + 1]);
            match (header_opt, sep_opt) {
                (Some(header_raw), Some(sep_raw)) if is_separator_row(&sep_raw) => {
                    let header: Vec<String> = header_raw.iter().map(|c| clean_cell(c)).collect();
                    let num_cols = header.len();
                    i += 2;

                    let mut data_rows = Vec::new();
                    while i < lines.len() {
                        if let Some(raw_cells) = parse_line_raw_cells(lines[i]) {
                            let mut cells: Vec<String> =
                                raw_cells.iter().map(|c| clean_cell(c)).collect();
                            if cells.len() < num_cols {
                                cells.resize(num_cols, String::new());
                            } else {
                                cells.truncate(num_cols);
                            }
                            data_rows.push(cells);
                            i += 1;
                        } else {
                            break;
                        }
                    }

                    let numeric: Vec<bool> = (0..num_cols)
                        .map(|col_idx| {
                            let mut has_non_empty = false;
                            let mut all_f64 = true;
                            for row in &data_rows {
                                if let Some(cell_str) = row.get(col_idx) {
                                    let cell = cell_str.trim();
                                    if !cell.is_empty() {
                                        has_non_empty = true;
                                        if cell.parse::<f64>().is_err() {
                                            all_f64 = false;
                                        }
                                    }
                                }
                            }
                            has_non_empty && all_f64
                        })
                        .collect();

                    let box_lines = render_box(&header, &data_rows, &numeric);
                    output.extend(box_lines);
                    continue;
                }
                _ => {}
            }
        }

        output.push(lines[i].to_string());
        i += 1;
    }

    output.join("\n")
}

fn parse_line_raw_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let mut fields: Vec<&str> = line.split('|').map(str::trim).collect();
    if fields.first() == Some(&"") {
        fields.remove(0);
    }
    if fields.last() == Some(&"") {
        fields.pop();
    }
    if fields.is_empty() {
        None
    } else {
        Some(fields.into_iter().map(String::from).collect())
    }
}

fn is_separator_row(cells: &[String]) -> bool {
    if cells.is_empty() {
        return false;
    }
    cells.iter().all(|c| is_separator_cell(c))
}

fn is_separator_cell(cell: &str) -> bool {
    let cell = cell.trim();
    if cell.is_empty() {
        return false;
    }
    let mut s = cell;
    if s.starts_with(':') {
        s = &s[1..];
    }
    if s.ends_with(':') {
        s = &s[..s.len() - 1];
    }
    !s.is_empty() && s.chars().all(|c| c == '-')
}
