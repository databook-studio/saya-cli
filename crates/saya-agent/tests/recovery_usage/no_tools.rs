struct NoTools {
    calls: Arc<Mutex<usize>>,
}
#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        *self.calls.lock().unwrap() += 1;
        Ok(serde_json::Value::Null)
    }
}
