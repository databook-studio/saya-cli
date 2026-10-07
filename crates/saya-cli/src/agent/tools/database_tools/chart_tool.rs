use saya_agent::{ToolError, ToolExecutionContext};

use super::DatabaseTools;
use crate::connection::OperationEntry;

impl DatabaseTools {
    /// Runs `sql` read-only at the full row cap, renders an interactive chart file with the
    /// requested type, opens it, and returns the file path (never the rows).
    pub(super) async fn render_chart(
        &self,
        arguments: &serde_json::Value,
        operation: &OperationEntry<'_>,
        context: &ToolExecutionContext,
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
        // The replacement precondition parses like `workspace_write`'s: an
        // explicit null counts as absent (the create form). Validation has
        // already typed a present non-null digest as a string and refused a
        // digest without `save_to`; the parse stays fail-closed anyway.
        let expected_digest = match arguments.get("expected_digest") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or(ToolError::ExpectedDigestNotString)?
                    .to_owned(),
            ),
        };
        let sql = arguments
            .get("sql")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::InvalidQueryArguments)?;
        let result = crate::agent::state_tools::operation_execute(
            operation,
            saya_types::QueryRequest::new(sql, self.max_rows),
            context,
        )
        .await?;
        if context.cancellation.is_cancelled() {
            return Err(ToolError::QueryFailedDetail("query cancelled".into()));
        }

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
            // Publish through the guarded primitives `workspace_write` uses,
            // BEFORE the temp copy and the browser open: a refused save opens
            // nothing. No redaction-marker guard applies — the content is
            // rendered from the query result, not model-supplied text — and
            // no checkpoint capture (D5): a chart is a database-derived
            // artifact, not a direct-file checkpoint.
            super::chart_save::publish_chart_html(
                workspace.ok_or(ToolError::WorkspaceUnavailable)?,
                path,
                html.as_bytes(),
                expected_digest.as_deref(),
            )?;
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
