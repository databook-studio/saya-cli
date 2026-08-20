use saya_agent::ToolError;

use super::DatabaseTools;

impl DatabaseTools {
    /// Runs `sql` read-only at the full row cap, renders an interactive chart file with the
    /// requested type, opens it, and returns the file path (never the rows).
    pub(super) async fn render_chart(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let sql = arguments
            .get("sql")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::InvalidQueryArguments)?;
        let connection = arguments.get("connection").and_then(|v| v.as_str());
        let entry = self.registry.resolve(connection)?;
        let result = entry
            .connector
            .execute(saya_types::QueryRequest::new(sql, self.max_rows))
            .await
            .map_err(|_| ToolError::QueryFailed)?;

        let mut spec = crate::chart::suggest_spec(&result);
        if let Some(kind) = arguments
            .get("chart_type")
            .and_then(serde_json::Value::as_str)
            .and_then(crate::chart::ChartKind::parse)
        {
            spec.kind = kind;
        }
        if let Some(x) = arguments.get("x").and_then(serde_json::Value::as_str) {
            spec.x = Some(x.to_string());
        }
        if let Some(y) = arguments.get("y").and_then(|v| v.as_array()) {
            let cols: Vec<String> = y
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if !cols.is_empty() {
                spec.y = cols;
            }
        }
        if let Some(title) = arguments.get("title").and_then(serde_json::Value::as_str) {
            spec.title = Some(title.to_string());
        }

        let html = crate::chart::render_html(&result, &spec).map_err(ToolError::Chart)?;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("saya-chart-{unique}.html"));
        crate::chart::write_html(&html, &path).map_err(ToolError::Chart)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let _ = crate::chart::open_file(&path);
        Ok(serde_json::json!({
            "path": path.display().to_string(),
            "note": "Interactive chart written and opened in the browser."
        }))
    }
}
