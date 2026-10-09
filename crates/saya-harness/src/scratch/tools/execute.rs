use async_trait::async_trait;
use saya_agent::{CancellationToken, ToolError, ToolExecutionContext, ToolExecutor};
use saya_types::QueryResult;

use crate::scratch::validate::validate;
use crate::scratch::{ImportError, ScratchError};

use super::{SCRATCH_IMPORT_TOOL, SCRATCH_SQL_TOOL, ScratchSql};

impl ScratchSql {
    /// Executes one statement: validation first (the validator is the thing
    /// under test — its refusal happens before any statement reaches DuckDB),
    /// then the bounded engine run under a fresh, never-cancelled token.
    pub async fn run(&self, sql: &str) -> Result<QueryResult, ScratchError> {
        self.run_cancellable(sql, &CancellationToken::new()).await
    }

    /// Executes one statement under the caller's turn token: already
    /// cancelled, the statement starts not at all; cancelled mid-run, the
    /// engine is interrupted and joined before the cancelled error surfaces.
    pub async fn run_cancellable(
        &self,
        sql: &str,
        cancellation: &CancellationToken,
    ) -> Result<QueryResult, ScratchError> {
        let validated = validate(sql)?;
        self.db.execute(validated, sql, cancellation).await
    }

    /// One `scratch_sql` call: argument gate, the statement run under the
    /// caller's cancellation, and the honest serialization.
    async fn sql_call(
        &self,
        arguments: &serde_json::Map<String, serde_json::Value>,
        cancellation: &CancellationToken,
    ) -> Result<serde_json::Value, ToolError> {
        if arguments.len() != 1 || !arguments.contains_key("sql") {
            return Err(ToolError::UnsupportedProperty);
        }
        let sql = arguments
            .get("sql")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::SqlNotString)?;
        let result = self
            .run_cancellable(sql, cancellation)
            .await
            .map_err(scratch_tool_error)?;
        serde_json::to_value(result).map_err(|_| ToolError::QueryResultUnavailable)
    }
}

#[async_trait]
impl ToolExecutor for ScratchSql {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let arguments = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
        match name {
            SCRATCH_SQL_TOOL => self.sql_call(arguments, &CancellationToken::new()).await,
            SCRATCH_IMPORT_TOOL => {
                let result = self
                    .import(arguments, &CancellationToken::new())
                    .await
                    .map_err(scratch_tool_error)?;
                serde_json::to_value(result).map_err(|_| ToolError::QueryResultUnavailable)
            }
            _ => Err(ToolError::UnsupportedTool),
        }
    }

    /// The loop's entry point: `scratch_sql` and `scratch_import` ride the
    /// turn's cancellation token; every other name keeps the plain `execute`
    /// path.
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        match name {
            SCRATCH_SQL_TOOL => {
                let arguments = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
                self.sql_call(arguments, &context.cancellation).await
            }
            SCRATCH_IMPORT_TOOL => {
                let arguments = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
                let result = self
                    .import(arguments, &context.cancellation)
                    .await
                    .map_err(scratch_tool_error)?;
                serde_json::to_value(result).map_err(|_| ToolError::QueryResultUnavailable)
            }
            _ => self.execute(name, arguments).await,
        }
    }
}

fn scratch_tool_error(error: ScratchError) -> ToolError {
    match error {
        ScratchError::TimedOut | ScratchError::Import(ImportError::TimedOut) => {
            ToolError::QueryTimedOut
        }
        other => ToolError::QueryFailedDetail(other.to_string()),
    }
}
