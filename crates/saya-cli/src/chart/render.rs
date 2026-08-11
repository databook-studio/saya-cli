//! Chart.js HTML rendering.

use saya_types::QueryResult;

use super::{ChartKind, ChartSpec, cell_to_f64, is_numeric_column, normalize_row};

#[allow(dead_code)]
const PALETTE: &[&str] = &[
    "#9d8bf5", "#6a9bcc", "#7fae6b", "#e0a458", "#e5695f", "#57c7c7", "#c98bd6", "#d4a27f",
];

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_json_for_script(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u0026"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

fn cell_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
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
    let escaped_config_json = escape_json_for_script(&config_json);

    let inlined_chartjs = include_str!("../assets/chart.umd.min.js");
    let doc_title = spec.title.as_deref().unwrap_or("saya chart");
    let escaped_title = html_escape(doc_title);

    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\n\
        <title>{escaped_title}</title>\n\
        <style>html,body{{margin:0;height:100%;background:#1e1c24}}\n\
        #wrap{{box-sizing:border-box;height:100%;padding:24px}}</style>\n\
        <script>{inlined_chartjs}</script></head>\n\
        <body><div id=\"wrap\"><canvas id=\"c\"></canvas></div>\n\
        <script id=\"saya-chart-config\" type=\"application/json\">{escaped_config_json}</script>\n\
        <script>const CONFIG=JSON.parse(document.getElementById('saya-chart-config').textContent);new Chart(document.getElementById('c').getContext('2d'),CONFIG);</script>\n\
        </body></html>"
    );

    Ok(html)
}
