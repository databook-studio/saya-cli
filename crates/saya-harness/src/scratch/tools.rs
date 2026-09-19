//! The `scratch_sql` tool: the run's one writable SQL surface (ADR 0003).
//!
//! Admitted only when the run's approved capabilities include the `scratch`
//! scope — hidden-not-advertised, the same pattern the other run tools use:
//! without the scope the tool is never constructed and never advertised, so
//! nothing is denied at call time that was offered at call time. It is not a
//! `DatabaseConnector` and never enters the `ConnectionRegistry`: a scratch
//! write has no type-level path to any user database.

use std::{path::Path, time::Duration};

use async_trait::async_trait;
use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect, ToolError, ToolExecutor};
use saya_types::{Capabilities, QueryResult};

use super::ScratchError;
use super::open::ScratchDb;
use super::validate::{MAX_SQL_BYTES, validate};

/// The tool's name in the run engine's toolset.
pub const SCRATCH_SQL_TOOL: &str = "scratch_sql";

/// The admitted `scratch_sql` tool over one run's opened scratch database.
pub struct ScratchSql {
    db: ScratchDb,
}

impl ScratchSql {
    /// Admits the tool only when the run's capabilities include the `scratch`
    /// scope. A refused admission opens nothing — the scratch file is not
    /// touched — and returns `None` for the run toolset to simply not
    /// advertise.
    pub fn admit(
        run_root: &Path,
        capabilities: &Capabilities,
    ) -> Result<Option<Self>, ScratchError> {
        if !capabilities.scratch {
            return Ok(None);
        }
        Ok(Some(Self {
            db: ScratchDb::open(run_root)?,
        }))
    }

    /// Opens the tool over a root the caller has already gated: the
    /// interactive session's `sessions/<id>/`, where admission is per call
    /// (the approval engine asks; nothing is pre-declared to admit). The
    /// file, configuration, and 0600 hardening are the pinned scratch
    /// semantics either way.
    pub fn open(root: &Path) -> Result<Self, ScratchError> {
        Ok(Self {
            db: ScratchDb::open(root)?,
        })
    }

    /// Narrows the per-statement timeout, for the run engine to follow a
    /// step's remaining budget. Never called with a longer value.
    pub fn with_query_timeout(mut self, query_timeout: Duration) -> Self {
        self.db = self.db.with_query_timeout(query_timeout);
        self
    }

    /// The tool definition. The write is plan-gated, not per-call approved:
    /// the scope is approved once at plan approval, so `requires_approval`
    /// stays false and admission above is what refuses the tool. The declared
    /// effect is the honest write-shaped local state — the run's own scratch
    /// file — which read-only approval denies and the loop refuses unless the
    /// runner was constructed with the write permission on.
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: SCRATCH_SQL_TOOL.to_string(),
            description: "Run one statement against this run's scratch database — the run's \
                only writable SQL. It holds the run's staged intermediate results: CREATE \
                TABLE, INSERT, UPDATE, DELETE, and SELECT over them, joins and scoring \
                included. Single statement per call; results are capped at 50 rows. No file \
                reads of any kind — read_csv, read_parquet, ATTACH, COPY, INSTALL and LOAD \
                are refused — so stage corpus data through the workspace tools first."
                .into(),
            read_only: false,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "sql": { "type": "string", "maxLength": MAX_SQL_BYTES }
                },
                "required": ["sql"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                // Scratch is run-scoped state, never a user-registered database.
                database_data: false,
                // No egress: external access is off and locked at open.
                external_side_effect: false,
                // Plan-gated, not per-call approved (ADR 0003).
                requires_approval: false,
                // The write-shaped local state the effect machinery already
                // knows how to gate.
                local_state: LocalStateEffect::WriteWorkspace,
            },
            completion: Some("scratch SQL executed".into()),
        }
    }

    /// The tool's definitions when — and only when — the scratch scope is
    /// approved; empty (hidden, not advertised as always-empty) otherwise.
    pub fn definitions(capabilities: &Capabilities) -> Vec<ToolDefinition> {
        if capabilities.scratch {
            vec![Self::definition()]
        } else {
            Vec::new()
        }
    }

    /// Executes one statement: validation first (the validator is the thing
    /// under test — its refusal happens before any statement reaches DuckDB),
    /// then the bounded engine run.
    pub async fn run(&self, sql: &str) -> Result<QueryResult, ScratchError> {
        let validated = validate(sql)?;
        self.db.execute(validated, sql).await
    }
}

#[async_trait]
impl ToolExecutor for ScratchSql {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if name != SCRATCH_SQL_TOOL {
            return Err(ToolError::UnsupportedTool);
        }
        let arguments = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
        if arguments.len() != 1 || !arguments.contains_key("sql") {
            return Err(ToolError::UnsupportedProperty);
        }
        let sql = arguments
            .get("sql")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::SqlNotString)?;
        let result = self.run(sql).await.map_err(|error| match &error {
            ScratchError::TimedOut => ToolError::QueryTimedOut,
            other => ToolError::QueryFailedDetail(other.to_string()),
        })?;
        serde_json::to_value(result).map_err(|_| ToolError::QueryResultUnavailable)
    }
}
