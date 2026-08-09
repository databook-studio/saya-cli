//! Renders a query result as a text horizontal bar chart for the transcript,
//! or generates self-contained interactive Chart.js HTML documents.

use saya_types::QueryResult;

const BAR_WIDTH: usize = 40; // longest bar in cells
const MAX_ROWS: usize = 20; // rows charted before truncating

#[allow(dead_code)]
const PALETTE: &[&str] = &[
    "#9d8bf5", "#6a9bcc", "#7fae6b", "#e0a458", "#e5695f", "#57c7c7", "#c98bd6", "#d4a27f",
];

/// Supported chart kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ChartKind {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
}

impl ChartKind {
    /// Parses a case-insensitive kind name; returns None if unrecognized.
    #[allow(dead_code)]
    pub(crate) fn parse(s: &str) -> Option<ChartKind> {
        match s.to_lowercase().as_str() {
            "bar" => Some(ChartKind::Bar),
            "line" => Some(ChartKind::Line),
            "area" => Some(ChartKind::Area),
            "pie" => Some(ChartKind::Pie),
            "doughnut" => Some(ChartKind::Doughnut),
            "scatter" => Some(ChartKind::Scatter),
            _ => None,
        }
    }

    #[allow(dead_code)]
    fn chartjs_type(self) -> &'static str {
        match self {
            ChartKind::Bar => "bar",
            ChartKind::Line | ChartKind::Area => "line",
            ChartKind::Pie => "pie",
            ChartKind::Doughnut => "doughnut",
            ChartKind::Scatter => "scatter",
        }
    }
}

/// Which columns to plot and how.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ChartSpec {
    pub(crate) kind: ChartKind,
    pub(crate) x: Option<String>, // label / x-axis column name; None => auto
    pub(crate) y: Vec<String>,    // value column name(s); empty => auto
    pub(crate) title: Option<String>,
}

fn is_numeric_column(rows: &[Vec<serde_json::Value>], col_idx: usize) -> bool {
    let mut non_null_count = 0;
    for row in rows {
        if let Some(cell) = row.get(col_idx).filter(|c| !c.is_null()) {
            non_null_count += 1;
            if !cell.is_number() {
                return false;
            }
        }
    }
    non_null_count >= 1
}

fn cell_to_f64(value: &serde_json::Value) -> Option<f64> {
    value.as_f64()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Heuristic default when the caller doesn't specify how to chart `result`.
#[allow(dead_code)]
pub(crate) fn suggest_spec(result: &QueryResult) -> ChartSpec {
    let col_count = result.columns.len();
    if col_count == 0 || result.rows.is_empty() {
        return ChartSpec {
            kind: ChartKind::Bar,
            x: None,
            y: Vec::new(),
            title: None,
        };
    }

    let normalized_rows: Vec<Vec<serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| normalize_row(row, col_count))
        .collect();

    let numeric: Vec<usize> = (0..col_count)
        .filter(|&idx| is_numeric_column(&normalized_rows, idx))
        .collect();
    let categorical: Vec<usize> = (0..col_count)
        .filter(|idx| !numeric.contains(idx))
        .collect();

    if col_count == 2 && numeric.len() == 2 {
        ChartSpec {
            kind: ChartKind::Scatter,
            x: Some(result.columns[0].clone()),
            y: vec![result.columns[1].clone()],
            title: None,
        }
    } else if !categorical.is_empty() && !numeric.is_empty() {
        ChartSpec {
            kind: ChartKind::Bar,
            x: Some(result.columns[categorical[0]].clone()),
            y: vec![result.columns[numeric[0]].clone()],
            title: None,
        }
    } else {
        let x_col = result.columns[0].clone();
        let y_col = if let Some(&idx) = numeric.first() {
            result.columns[idx].clone()
        } else if result.columns.len() > 1 {
            result.columns[1].clone()
        } else {
            result.columns[0].clone()
        };
        ChartSpec {
            kind: ChartKind::Bar,
            x: Some(x_col),
            y: vec![y_col],
            title: None,
        }
    }
}

/// Builds a self-contained HTML document string containing an interactive Chart.js chart.
#[allow(dead_code)]
pub(crate) fn render_html(result: &QueryResult, spec: &ChartSpec) -> Result<String, String> {
    if result.rows.is_empty() {
        return Err("no rows to chart".into());
    }

    let col_count = result.columns.len();
    let normalized_rows: Vec<Vec<serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| normalize_row(row, col_count))
        .collect();

    // Resolve y column indices
    let mut y_indices = Vec::new();
    for name in &spec.y {
        if let Some(idx) = result.columns.iter().position(|c| c == name) {
            y_indices.push(idx);
        }
    }
    if y_indices.is_empty() {
        y_indices.extend((0..col_count).find(|&i| is_numeric_column(&normalized_rows, i)));
    }
    if y_indices.is_empty() {
        return Err("no numeric column to chart".into());
    }

    // Resolve x column index
    let x_index = spec
        .x
        .as_ref()
        .and_then(|x_name| result.columns.iter().position(|c| c == x_name))
        .unwrap_or_else(|| (0..col_count).find(|i| !y_indices.contains(i)).unwrap_or(0));

    let data_obj = match spec.kind {
        ChartKind::Scatter => {
            let datasets: Vec<serde_json::Value> = y_indices
                .iter()
                .enumerate()
                .map(|(i, &y_idx)| {
                    let y_name = result.columns.get(y_idx).map(|s| s.as_str()).unwrap_or("");
                    let mut points = Vec::new();
                    for row in &normalized_rows {
                        let x_val = row.get(x_index).and_then(cell_to_f64);
                        let y_val = row.get(y_idx).and_then(cell_to_f64);
                        if let (Some(x_f), Some(y_f)) = (x_val, y_val) {
                            points.push(serde_json::json!({ "x": x_f, "y": y_f }));
                        }
                    }
                    serde_json::json!({
                        "label": y_name,
                        "data": points,
                        "backgroundColor": PALETTE[i % PALETTE.len()]
                    })
                })
                .collect();
            serde_json::json!({ "datasets": datasets })
        }
        ChartKind::Pie | ChartKind::Doughnut => {
            let labels: Vec<String> = normalized_rows
                .iter()
                .map(|row| cell_to_string(&row[x_index]))
                .collect();
            let first_y = y_indices[0];
            let data: Vec<serde_json::Value> = normalized_rows
                .iter()
                .map(|row| {
                    row.get(first_y)
                        .and_then(cell_to_f64)
                        .map_or(serde_json::Value::Null, |f| serde_json::json!(f))
                })
                .collect();
            let bg_colors: Vec<&str> = (0..normalized_rows.len())
                .map(|i| PALETTE[i % PALETTE.len()])
                .collect();
            let dataset = serde_json::json!({
                "data": data,
                "backgroundColor": bg_colors
            });
            serde_json::json!({
                "labels": labels,
                "datasets": [dataset]
            })
        }
        ChartKind::Bar | ChartKind::Line | ChartKind::Area => {
            let labels: Vec<String> = normalized_rows
                .iter()
                .map(|row| cell_to_string(&row[x_index]))
                .collect();
            let datasets: Vec<serde_json::Value> = y_indices
                .iter()
                .enumerate()
                .map(|(i, &y_idx)| {
                    let y_name = result.columns.get(y_idx).map(|s| s.as_str()).unwrap_or("");
                    let data: Vec<serde_json::Value> = normalized_rows
                        .iter()
                        .map(|row| {
                            row.get(y_idx)
                                .and_then(cell_to_f64)
                                .map_or(serde_json::Value::Null, |f| serde_json::json!(f))
                        })
                        .collect();
                    let color = PALETTE[i % PALETTE.len()];
                    let mut ds = serde_json::json!({
                        "label": y_name,
                        "data": data,
                        "backgroundColor": color,
                        "borderColor": color,
                        "borderWidth": 2
                    });
                    if spec.kind == ChartKind::Line {
                        ds["tension"] = serde_json::json!(0.3);
                    } else if spec.kind == ChartKind::Area {
                        ds["fill"] = serde_json::json!(true);
                        ds["tension"] = serde_json::json!(0.3);
                    }
                    ds
                })
                .collect();
            serde_json::json!({
                "labels": labels,
                "datasets": datasets
            })
        }
    };

    let mut options = serde_json::json!({
        "responsive": true,
        "plugins": {
            "legend": {
                "labels": { "color": "#e8e6dc" }
            },
            "title": {
                "display": spec.title.is_some(),
                "text": spec.title.as_deref().unwrap_or(""),
                "color": "#e8e6dc"
            }
        }
    });

    if spec.kind != ChartKind::Pie && spec.kind != ChartKind::Doughnut {
        options["scales"] = serde_json::json!({
            "x": {
                "ticks": { "color": "#b0aea5" },
                "grid": { "color": "rgba(255,255,255,0.08)" }
            },
            "y": {
                "ticks": { "color": "#b0aea5" },
                "grid": { "color": "rgba(255,255,255,0.08)" }
            }
        });
    }

    let config = serde_json::json!({
        "type": spec.kind.chartjs_type(),
        "data": data_obj,
        "options": options
    });

    let config_json = serde_json::to_string(&config)
        .map_err(|e| format!("failed to serialize chart config: {e}"))?;

    let inlined_chartjs = include_str!("assets/chart.umd.min.js");
    let doc_title = spec.title.as_deref().unwrap_or("saya chart");
    let escaped_title = html_escape(doc_title);

    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\n\
        <title>{escaped_title}</title>\n\
        <style>html,body{{margin:0;height:100%;background:#1e1c24}}\n\
        #wrap{{box-sizing:border-box;height:100%;padding:24px}}</style>\n\
        <script>{inlined_chartjs}</script></head>\n\
        <body><div id=\"wrap\"><canvas id=\"c\"></canvas></div>\n\
        <script>const CONFIG={config_json};new Chart(document.getElementById('c').getContext('2d'),CONFIG);</script>\n\
        </body></html>"
    );

    Ok(html)
}

/// Writes the HTML document string to the specified path.
#[allow(dead_code)]
pub(crate) fn write_html(html: &str, path: &std::path::Path) -> Result<(), String> {
    std::fs::write(path, html).map_err(|e| format!("failed to write chart file: {e}"))
}

/// Renders `result` as a horizontal bar chart: a label column and a numeric value
/// column, one bar per row. Returns Err when there is no numeric column to plot.
pub(crate) fn format_bar_chart(result: &QueryResult) -> Result<String, String> {
    let col_count = result.columns.len();
    if col_count == 0 {
        return Err("no numeric column to chart — try a query that returns a number".into());
    }

    let normalized_rows: Vec<Vec<serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| normalize_row(row, col_count))
        .collect();

    // Determine, per column, whether it is NUMERIC:
    // it has >= 1 non-null cell and EVERY non-null cell is a serde_json::Value::Number.
    // value_col = index of the first numeric column.
    let mut value_col = None;
    for col_idx in 0..col_count {
        let mut non_null_count = 0;
        let mut all_numbers = true;
        for row in &normalized_rows {
            let cell = &row[col_idx];
            if !cell.is_null() {
                non_null_count += 1;
                if !cell.is_number() {
                    all_numbers = false;
                    break;
                }
            }
        }
        if non_null_count >= 1 && all_numbers {
            value_col = Some(col_idx);
            break;
        }
    }

    let value_col = match value_col {
        Some(idx) => idx,
        None => {
            return Err("no numeric column to chart — try a query that returns a number".into());
        }
    };

    // label_col = the first column index that is not value_col
    // (prefer any column != value_col; if the result has only one column, use value_col itself)
    let label_col = (0..col_count)
        .find(|&c| c != value_col)
        .unwrap_or(value_col);

    let value_col_name = result
        .columns
        .get(value_col)
        .map(|s| s.as_str())
        .unwrap_or("");
    let label_col_name = result
        .columns
        .get(label_col)
        .map(|s| s.as_str())
        .unwrap_or("");

    let charted_rows = normalized_rows.iter().take(MAX_ROWS);

    struct RowData {
        label_raw: String,
        value_f64: f64,
        value_str: String,
    }

    let mut row_data_list = Vec::with_capacity(MAX_ROWS.min(normalized_rows.len()));
    let mut max_value_f64: f64 = 0.0;
    let mut max_label_len: usize = 0;

    for row in charted_rows {
        let label_cell = &row[label_col];
        let value_cell = &row[value_col];

        let label_raw = cell_to_string(label_cell);
        let value_f64 = value_cell.as_f64().unwrap_or(0.0);
        let value_str = cell_to_string(value_cell);

        let label_len = label_raw.chars().count();
        if label_len > max_label_len {
            max_label_len = label_len;
        }

        if value_f64.max(0.0) > max_value_f64 {
            max_value_f64 = value_f64.max(0.0);
        }

        row_data_list.push(RowData {
            label_raw,
            value_f64,
            value_str,
        });
    }

    // max_value = the maximum of value.max(0.0) across charted rows; if 0.0, set to 1.0
    let max_value = if max_value_f64 == 0.0 {
        1.0
    } else {
        max_value_f64
    };

    // label_width = min(24, max label char count across charted rows), at least 1
    let label_width = max_label_len.clamp(1, 24);

    let mut lines = Vec::new();
    lines.push(format!("chart: {value_col_name} by {label_col_name}"));

    for row in row_data_list {
        let truncated_label = truncate_label(&row.label_raw, label_width);
        let val_pos = row.value_f64.max(0.0);
        let bar_len = ((val_pos / max_value) * BAR_WIDTH as f64).round() as usize;
        let bar = "█".repeat(bar_len);

        lines.push(format!(
            "{truncated_label:<label_width$} │ {bar} {}",
            row.value_str
        ));
    }

    if result.rows.len() > MAX_ROWS {
        lines.push(format!(
            "… (showing first {MAX_ROWS} of {} rows)",
            result.rows.len()
        ));
    }

    Ok(lines.join("\n"))
}

fn normalize_row(row: &serde_json::Value, col_count: usize) -> Vec<serde_json::Value> {
    let mut cells = match row {
        serde_json::Value::Array(arr) => arr.clone(),
        scalar => vec![scalar.clone()],
    };
    cells.resize(col_count, serde_json::Value::Null);
    cells
}

fn cell_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn truncate_label(label: &str, label_width: usize) -> String {
    let char_count = label.chars().count();
    if char_count <= label_width {
        label.to_string()
    } else if label_width <= 1 {
        "…".to_string()
    } else {
        let mut truncated: String = label.chars().take(label_width - 1).collect();
        truncated.push('…');
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_chart_text_label_and_numeric() {
        let result = QueryResult {
            columns: vec!["city".to_string(), "pop".to_string()],
            rows: vec![
                json!(["Tokyo", 37.4]),
                json!(["Delhi", 29.3]),
                json!(["Shanghai", 26.3]),
            ],
            row_count: 3,
            truncated: false,
            executed_sql: "SELECT city, pop FROM cities".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("chart: pop by city"));
        assert!(chart.contains('█'));
        assert!(chart.contains("Tokyo"));
        assert!(chart.contains("Delhi"));
        assert!(chart.contains("Shanghai"));

        // Largest value ("Tokyo" with 37.4) should have the longest bar (40 '█' chars)
        let tokyo_line = chart.lines().find(|l| l.contains("Tokyo")).unwrap();
        let tokyo_bar_count = tokyo_line.chars().filter(|&c| c == '█').count();
        assert_eq!(tokyo_bar_count, 40);

        let delhi_line = chart.lines().find(|l| l.contains("Delhi")).unwrap();
        let delhi_bar_count = delhi_line.chars().filter(|&c| c == '█').count();
        assert!(delhi_bar_count < 40);
    }

    #[test]
    fn test_chart_no_numeric_column() {
        let result = QueryResult {
            columns: vec!["name".to_string(), "city".to_string()],
            rows: vec![json!(["Alice", "Tokyo"]), json!(["Bob", "Delhi"])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT name, city FROM users".to_string(),
        };
        let err = format_bar_chart(&result).unwrap_err();
        assert_eq!(
            err,
            "no numeric column to chart — try a query that returns a number"
        );
    }

    #[test]
    fn test_chart_value_label_formatting() {
        let result = QueryResult {
            columns: vec!["item".to_string(), "price".to_string()],
            rows: vec![json!(["apple", 4.99]), json!(["banana", 0.5])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT item, price FROM items".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("4.99"));
        assert!(chart.contains("0.5"));
    }

    #[test]
    fn test_chart_single_numeric_column() {
        let result = QueryResult {
            columns: vec!["score".to_string()],
            rows: vec![json!([10]), json!([20])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT score FROM scores".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("chart: score by score"));
        assert!(chart.contains("10"));
        assert!(chart.contains("20"));
    }

    #[test]
    fn test_chart_truncation_footer() {
        let mut rows = Vec::new();
        for i in 0..25 {
            rows.push(json!([format!("label_{i}"), i]));
        }
        let result = QueryResult {
            columns: vec!["label".to_string(), "val".to_string()],
            rows,
            row_count: 25,
            truncated: false,
            executed_sql: "SELECT label, val FROM test".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("… (showing first 20 of 25 rows)"));
    }
}

#[cfg(test)]
mod chart_gen_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_chart_kind_parse() {
        assert_eq!(ChartKind::parse("bar"), Some(ChartKind::Bar));
        assert_eq!(ChartKind::parse("BAR"), Some(ChartKind::Bar));
        assert_eq!(ChartKind::parse("Line"), Some(ChartKind::Line));
        assert_eq!(ChartKind::parse("AREA"), Some(ChartKind::Area));
        assert_eq!(ChartKind::parse("pie"), Some(ChartKind::Pie));
        assert_eq!(ChartKind::parse("Doughnut"), Some(ChartKind::Doughnut));
        assert_eq!(ChartKind::parse("scatter"), Some(ChartKind::Scatter));
        assert_eq!(ChartKind::parse("unknown"), None);
        assert_eq!(ChartKind::parse("123"), None);
    }

    #[test]
    fn test_suggest_spec() {
        // text + numeric => Bar
        let text_num_result = QueryResult {
            columns: vec!["category".to_string(), "count".to_string()],
            rows: vec![json!(["A", 10]), json!(["B", 20])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT category, count FROM t".to_string(),
        };
        let spec = suggest_spec(&text_num_result);
        assert_eq!(spec.kind, ChartKind::Bar);
        assert_eq!(spec.x, Some("category".to_string()));
        assert_eq!(spec.y, vec!["count".to_string()]);
        assert_eq!(spec.title, None);

        // two numeric => Scatter
        let two_num_result = QueryResult {
            columns: vec!["x_val".to_string(), "y_val".to_string()],
            rows: vec![json!([1.0, 2.0]), json!([3.0, 4.0])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT x_val, y_val FROM t".to_string(),
        };
        let spec2 = suggest_spec(&two_num_result);
        assert_eq!(spec2.kind, ChartKind::Scatter);
        assert_eq!(spec2.x, Some("x_val".to_string()));
        assert_eq!(spec2.y, vec!["y_val".to_string()]);
        assert_eq!(spec2.title, None);
    }

    #[test]
    fn test_render_html_bar_chart() {
        let result = QueryResult {
            columns: vec!["rating".to_string(), "films".to_string()],
            rows: vec![json!(["G", 178]), json!(["R", 195])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT rating, films FROM film".to_string(),
        };
        let spec = ChartSpec {
            kind: ChartKind::Bar,
            x: Some("rating".to_string()),
            y: vec!["films".to_string()],
            title: None,
        };
        let html = render_html(&result, &spec).unwrap();
        assert!(html.contains("\"type\":\"bar\""));
        assert!(html.contains("178"));
        assert!(html.contains("195"));
        assert!(html.contains("\"G\""));
        assert!(html.contains("\"R\""));
        assert!(html.contains("Chart.js v4.4.4"));

        let no_num_result = QueryResult {
            columns: vec!["col1".to_string(), "col2".to_string()],
            rows: vec![json!(["a", "b"]), json!(["c", "d"])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT col1, col2 FROM t".to_string(),
        };
        let err_spec = ChartSpec {
            kind: ChartKind::Bar,
            x: None,
            y: Vec::new(),
            title: None,
        };
        assert!(render_html(&no_num_result, &err_spec).is_err());
    }
}
