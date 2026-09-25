use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutor};

#[tokio::test]
async fn run_step_refuses_chart_save_without_its_workspace_write_permit() {
    let database = crate::agent::tools::DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let tools = super::run_tools::RunTools::compose(Arc::new(database), None, None, None);
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({"sql": "SELECT 1", "chart_type": "bar", "save_to": "chart.html"}),
        )
        .await
        .expect_err("a step without workspace-write cannot save a chart");
    assert_eq!(
        error,
        ToolError::WorkspaceWrite("workspace-write is not permitted for this step".into())
    );
}
