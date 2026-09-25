use saya_agent::ToolError;

use super::DatabaseTools;

impl DatabaseTools {
    /// Runs `sql` read-only at the full row cap, renders an interactive chart file with the
    /// requested type, opens it, and returns the file path (never the rows).
    pub(super) async fn render_chart(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let saved_path = arguments.get("save_to").and_then(serde_json::Value::as_str);
        let workspace = if saved_path.is_some() {
            Some(
                self.workspace
                    .as_ref()
                    .ok_or(ToolError::WorkspaceUnavailable)?,
            )
        } else {
            None
        };
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
            .map_err(|error| ToolError::QueryFailedDetail(error.to_string()))?;

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
        if let Some(path) = saved_path {
            if html.len() > saya_harness::workspace::MAX_IO_BYTES {
                return Err(ToolError::WorkspaceWrite(format!(
                    "content is over the {}-byte workspace write bound",
                    saya_harness::workspace::MAX_IO_BYTES
                )));
            }
            workspace
                .ok_or(ToolError::WorkspaceUnavailable)?
                .write(path, html.as_bytes())
                .map_err(|error| ToolError::WorkspaceWrite(error.to_string()))?;
        }
        let mut chart = crate::chart::reserve_temp_chart().map_err(ToolError::Chart)?;
        chart.write_html(&html).map_err(ToolError::Chart)?;
        let path = chart.path().to_path_buf();
        let _ = crate::chart::open_file(&path);
        let result_path = saved_path
            .map(str::to_owned)
            .unwrap_or_else(|| path.display().to_string());
        let note = if saved_path.is_some() {
            "The same chart HTML was saved to the workspace path and opened from a private temporary copy."
        } else {
            "Interactive chart written to a private temporary file and opened in the browser."
        };
        Ok(serde_json::json!({
            "path": result_path,
            "note": note
        }))
    }
}
