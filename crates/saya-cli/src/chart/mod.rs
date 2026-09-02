//! Generates self-contained interactive Chart.js HTML documents.

mod kind;
mod render;
mod spec;

#[cfg(test)]
use saya_types::QueryResult;

pub(crate) use kind::{ChartKind, ChartSpec};
pub(crate) use render::render_html;
pub(crate) use spec::suggest_spec;

pub(super) fn is_numeric_column(rows: &[Vec<serde_json::Value>], col_idx: usize) -> bool {
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
pub(super) fn cell_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(_) => value.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

pub(super) fn normalize_row(row: &serde_json::Value, col_count: usize) -> Vec<serde_json::Value> {
    let mut cells = match row {
        serde_json::Value::Array(arr) => arr.clone(),
        scalar => vec![scalar.clone()],
    };
    cells.resize(col_count, serde_json::Value::Null);
    cells
}

/// Writes the HTML document string to the specified path.
#[allow(dead_code)]
pub(crate) fn write_html(html: &str, path: &std::path::Path) -> Result<(), String> {
    std::fs::write(path, html).map_err(|e| format!("failed to write chart file: {e}"))
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

    #[test]
    fn test_render_html_script_injection_prevention() {
        let result = QueryResult {
            columns: vec!["label".to_string(), "val".to_string()],
            rows: vec![
                json!(["</script><img src=x onerror=alert(1)>", 10]),
                json!(["</SCRIPT>", 20]),
            ],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT label, val FROM t".to_string(),
        };
        let spec = ChartSpec {
            kind: ChartKind::Bar,
            x: Some("label".to_string()),
            y: vec!["val".to_string()],
            title: None,
        };
        let html = render_html(&result, &spec).unwrap();

        assert!(html.contains("id=\"saya-chart-config\""));
        assert!(html.contains("type=\"application/json\""));
        assert!(html.contains("JSON.parse"));

        assert!(!html.contains("</script><img"));
        assert!(!html.contains("</SCRIPT>"));

        assert!(html.contains("\\u003c/script"));
        assert!(html.contains("\\u003c/SCRIPT"));
    }
}
