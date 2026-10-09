use std::io;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};

use super::RunProgram;
use super::output;
use super::{ProgramOutcome, RUN_PROGRAM_TOOL, RunnerError, not_typed, validate_json_argv};

#[async_trait]
impl ToolExecutor for RunProgram {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if name != RUN_PROGRAM_TOOL {
            return Err(ToolError::UnsupportedTool);
        }
        let outcome = self
            .dispatch(arguments)
            .await
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        let mut value =
            serde_json::to_value(&outcome).map_err(|_| ToolError::Runner(render_failure()))?;
        match self.record(&outcome) {
            Ok(path) => {
                value["record_path"] = serde_json::json!(path);
            }
            Err(error) => {
                // A failed record is reported beside the result, never
                // pretended away and never fatal to the child's report.
                value["record_error"] = serde_json::json!(error.to_string());
            }
        }
        Ok(value)
    }
}

impl RunProgram {
    /// Persists the redacted outcome into the run workspace — the disk
    /// record the model's copy mirrors. Nothing unredacted exists to write.
    fn record(&self, outcome: &ProgramOutcome) -> Result<String, RunnerError> {
        output::record_outcome(&self.record_dir, outcome)
    }

    /// Parses the call into typed argv — every element of `args` a string,
    /// nothing that could become a command line — and runs it.
    async fn dispatch(&self, arguments: serde_json::Value) -> Result<ProgramOutcome, RunnerError> {
        let object = arguments.as_object().ok_or(not_typed(
            "the call must be an object: program, args, timeout_seconds",
        ))?;
        for key in object.keys() {
            if !matches!(key.as_str(), "program" | "args" | "timeout_seconds") {
                return Err(not_typed(
                    "unknown property; the call is program, args, timeout_seconds",
                ));
            }
        }
        let program = object
            .get("program")
            .and_then(serde_json::Value::as_str)
            .ok_or(not_typed("program must be a string"))?;
        let argv: Vec<String> = match object.get("args") {
            None => Vec::new(),
            Some(serde_json::Value::Array(items)) => {
                validate_json_argv(items)?;
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| not_typed("every element of args must be a string"))
                    })
                    .collect::<Result<_, _>>()?
            }
            Some(_) => return Err(not_typed("args must be an array of strings")),
        };
        let timeout_seconds = match object.get("timeout_seconds") {
            None => None,
            Some(serde_json::Value::Number(number)) => Some(
                number
                    .as_u64()
                    .ok_or(not_typed("timeout_seconds must be a positive integer"))?,
            ),
            Some(_) => return Err(not_typed("timeout_seconds must be a positive integer")),
        };
        self.run(program, &argv, timeout_seconds).await
    }
}

fn render_failure() -> String {
    RunnerError::RecordFailed {
        source: io::Error::new(
            io::ErrorKind::InvalidData,
            "the outcome could not be rendered",
        ),
    }
    .to_string()
}
