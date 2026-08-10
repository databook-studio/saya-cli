//! Generates self-contained interactive Chart.js HTML documents.

use saya_types::QueryResult;

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
            if cell_to_f64(cell).is_none() {
                return false;
            }
        }
    }
    non_null_count >= 1
}

/// Interprets a cell as an `f64`, accepting both JSON numbers and numeric
/// *strings*. Postgres `NUMERIC`/`DECIMAL` and MySQL `DECIMAL` (e.g. `SUM`/`AVG`,
/// money columns) decode to JSON strings, so a number-only check would silently
/// drop them from charts.
fn cell_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(_) => value.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
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

/// Opens `path` in the user's default application (browser for .html).
pub(crate) fn open_file(path: &std::path::Path) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    };
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", path.display()))
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

    // Postgres NUMERIC/DECIMAL and MySQL DECIMAL decode to JSON *strings*, not
    // numbers (e.g. SUM(amount) -> "3094.78"). They must still chart.
    #[test]
    fn numeric_string_values_are_plotted() {
        let result = QueryResult {
            columns: vec!["month".to_string(), "revenue".to_string()],
            rows: vec![
                json!(["2022-01", "3094.78"]),
                json!(["2022-02", "10164.97"]),
            ],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT month, SUM(amount) AS revenue FROM payment GROUP BY 1"
                .to_string(),
        };
        let spec = suggest_spec(&result);
        assert_eq!(spec.y, vec!["revenue".to_string()]);
        let html = render_html(&result, &spec).unwrap();
        assert!(html.contains("3094.78"), "numeric-string value was dropped");
        assert!(
            html.contains("10164.97"),
            "numeric-string value was dropped"
        );
    }

    #[test]
    fn suggest_spec_detects_numeric_string_columns() {
        // Two numeric columns encoded as strings should be recognized as numeric
        // (and so picked as a scatter), not treated as categorical text.
        let result = QueryResult {
            columns: vec!["length".to_string(), "avg_rate".to_string()],
            rows: vec![json!(["46", "2.59"]), json!(["47", "2.70"])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT length, AVG(rate) FROM film GROUP BY 1".to_string(),
        };
        assert_eq!(suggest_spec(&result).kind, ChartKind::Scatter);
    }
}
