use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};

use super::owner::Trace;

#[derive(Default)]
pub struct ExecutorLog {
    pub started: Mutex<Vec<String>>,
    pub finished: Mutex<Vec<String>>,
    pub calls: AtomicUsize,
}

pub struct RecordingExecutor {
    pub log: Arc<ExecutorLog>,
    pub trace: Trace,
    pub fail_name: Option<String>,
    pub fail_no_effect: Option<String>,
    pub delay_slow: bool,
}

impl RecordingExecutor {
    pub fn new(trace: Trace) -> (Self, Arc<ExecutorLog>) {
        let log = Arc::new(ExecutorLog::default());
        (
            Self {
                log: Arc::clone(&log),
                trace,
                fail_name: None,
                fail_no_effect: None,
                delay_slow: false,
            },
            log,
        )
    }
}

#[async_trait]
impl ToolExecutor for RecordingExecutor {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.log.calls.fetch_add(1, Ordering::SeqCst);
        self.log.started.lock().unwrap().push(name.to_owned());
        self.trace.lock().unwrap().push(format!("executor:{name}"));
        if self.delay_slow && name == "slow" {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.log.finished.lock().unwrap().push(name.to_owned());
        if self.fail_no_effect.as_deref() == Some(name) {
            return Err(ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                "refused".into(),
            ))));
        }
        if self.fail_name.as_deref() == Some(name) {
            Err(ToolError::UnsupportedTool)
        } else {
            Ok(serde_json::json!({"ok": true, "name": name}))
        }
    }
}
