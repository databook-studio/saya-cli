//! The SQL family's fact lines: the read-shaped query tools say *why* they
//! are low-risk — the target, the statement, and the enforced bounds stated
//! as facts (one parsed read-only statement, the model-facing row cap, the
//! timeout, redaction before rows reach the model). `render_chart` shares
//! the target seam but not the bounds: it writes a file and opens a browser,
//! which is why it always asks.

use serde_json::Value;

use super::{ApprovalFacts, body};

/// The named connection, absent-or-empty meaning "no connection named".
fn named_connection(arguments: &Value) -> Option<&str> {
    arguments
        .get("connection")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// The target the call runs against: the connection it names, "every
/// connected database" for the fan-out, or the turn's primary — which the
/// registry resolves a connectionless call to, the same connection the grant
/// suggestion names.
fn target_line(name: &str, connection: Option<&str>, primary: Option<&str>) -> String {
    match (name, connection) {
        ("bounded_sql_query_all", _) => "every connected database".to_string(),
        (_, Some(connection)) => connection.to_string(),
        (_, None) => match primary {
            Some(primary) => primary.to_string(),
            None => "the primary connection".to_string(),
        },
    }
}

/// The read-shaped SQL family's body. The bounds segments are conditional:
/// a bound the composition does not carry produces no segment, never a
/// placeholder number.
pub(super) fn sql_facts(
    name: &str,
    arguments: &Value,
    facts: &ApprovalFacts,
    primary: Option<&str>,
    session_line: Option<String>,
) -> Option<String> {
    let sql = arguments.get("sql").and_then(Value::as_str)?;
    let sql = crate::agent::tools::collapse_whitespace(sql);
    if sql.is_empty() {
        return None;
    }
    let mut lines = vec![
        format!(
            "  target: {}",
            target_line(name, named_connection(arguments), primary)
        ),
        format!("  sql: {sql}"),
    ];
    let mut bounds = String::from("  bounds: one parsed read-only statement");
    if facts.row_cap > 0 {
        bounds.push_str(&format!(" · ≤ {} rows to the model", facts.row_cap));
    }
    // The fan-out wraps every per-database query in its own 30 s ceiling
    // (`fan_out.rs`), so its stated timeout is that constant, not the
    // connector's configured one; the single tools state the connector
    // timeout the composition resolved.
    if name == "bounded_sql_query_all" {
        let fan_out = crate::agent::tools::DatabaseTools::FAN_OUT_QUERY_TIMEOUT.as_secs();
        bounds.push_str(&format!(" · ≤ {fan_out}s timeout per database"));
    } else if facts.sql_timeout_seconds > 0 {
        bounds.push_str(&format!(" · {}s timeout", facts.sql_timeout_seconds));
    }
    lines.push(bounds);
    lines.push("  rows are redacted before they reach the model".to_string());
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    Some(body(format!("{name} — read-only query"), lines))
}

/// `render_chart`'s fact lines: the statement, and the external side effect
/// that is why it always asks.
pub(super) fn chart_facts(arguments: &Value, primary: Option<&str>) -> Option<String> {
    let sql = arguments.get("sql").and_then(Value::as_str)?;
    let sql = crate::agent::tools::collapse_whitespace(sql);
    if sql.is_empty() {
        return None;
    }
    let mut lines = vec![
        format!(
            "  target: {}",
            target_line("render_chart", named_connection(arguments), primary)
        ),
        format!("  sql: {sql}"),
    ];
    if let Some(path) = arguments.get("save_to").and_then(Value::as_str) {
        lines.push(format!(
            "  save_to: {}",
            crate::agent::tools::collapse_whitespace(path)
        ));
        lines.push(
            "  the same HTML is saved there; the browser opens a private temporary copy"
                .to_string(),
        );
        lines.push("  an existing file at this path may be replaced".to_string());
    } else {
        lines.push(
            "  this is why it always asks: the chart file is written and opened in your \
             browser"
                .to_string(),
        );
    }
    Some(body(
        "render_chart — writes a chart file and opens your browser".to_string(),
        lines,
    ))
}
