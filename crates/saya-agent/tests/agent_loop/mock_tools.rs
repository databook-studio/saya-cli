struct MockTools {
    calls: Arc<Mutex<Vec<String>>>,
}
#[async_trait]
impl ToolExecutor for MockTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"rows": 1}))
    }
}
