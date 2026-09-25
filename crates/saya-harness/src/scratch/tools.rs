//! The `scratch_sql` tool: the run's one writable SQL surface (ADR 0003).
//!
//! Admitted only when the run's approved capabilities include the `scratch`
//! scope — hidden-not-advertised, the same pattern the other run tools use:
//! without the scope the tool is never constructed and never advertised, so
//! nothing is denied at call time that was offered at call time. It is not a
//! `DatabaseConnector` and never enters the `ConnectionRegistry`: a scratch
//! write has no type-level path to any user database.

use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect, ToolError, ToolExecutor};
use saya_types::{Capabilities, QueryResult};

use super::import::{MAX_IMPORT_FILE_BYTES, import_bytes};
use super::open::ScratchDb;
use super::validate::{MAX_SQL_BYTES, validate};
use super::{ImportError, ImportResult, ScratchError};
use crate::workspace::Workspace;

/// The tool's name in the run engine's toolset.
pub const SCRATCH_SQL_TOOL: &str = "scratch_sql";
/// The contained CSV staging tool's name.
pub const SCRATCH_IMPORT_TOOL: &str = "scratch_import";

/// The admitted `scratch_sql` tool over one run's opened scratch database.
#[derive(Clone)]
pub struct ScratchSql {
    db: ScratchDb,
    workspace: Option<Arc<Workspace>>,
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
            workspace: None,
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
            workspace: None,
        })
    }

    /// Narrows the per-statement timeout, for the run engine to follow a
    /// step's remaining budget. Never called with a longer value.
    pub fn with_query_timeout(mut self, query_timeout: Duration) -> Self {
        self.db = self.db.with_query_timeout(query_timeout);
        self
    }

    /// Binds the one workspace whose contained CSV files may be staged into
    /// this scratch database. Without it, import is hidden and refuses.
    pub fn with_workspace(mut self, workspace: Arc<Workspace>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Rebinds the workspace import root for a recomposed session. `None`
    /// clears a prior binding, so an unbound session cannot retain imports.
    pub fn rebind_workspace(mut self, workspace: Option<Arc<Workspace>>) -> Self {
        self.workspace = workspace;
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

    /// The importer is a scratch-state write, but it is available only when
    /// the composition also bound a workspace.
    pub fn import_definition() -> ToolDefinition {
        ToolDefinition {
            name: SCRATCH_IMPORT_TOOL.to_string(),
            description: "Load one CSV file from this run's bound workspace into one scratch \
                table. The path is relative to that workspace; absolute paths, escapes, \
                symlinks, non-regular files, and files over 32 MiB are refused. Every column \
                becomes VARCHAR. CSV parsing is bounded; scratch itself keeps external access \
                off, so this is the only file-to-scratch route."
                .into(),
            read_only: false,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "table": { "type": "string" },
                    "header": { "type": "boolean", "default": true },
                    "delimiter": { "type": "string", "minLength": 1, "maxLength": 1, "default": "," },
                    "if_exists": { "type": "string", "enum": ["fail", "replace"], "default": "fail" }
                },
                "required": ["path", "table"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            completion: Some("workspace CSV imported into scratch".into()),
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

    async fn import(
        &self,
        arguments: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<ImportResult, ScratchError> {
        if arguments.keys().any(|key| {
            !matches!(
                key.as_str(),
                "path" | "table" | "header" | "delimiter" | "if_exists"
            )
        }) {
            return Err(ImportError::InvalidArguments.into());
        }
        let workspace = self
            .workspace
            .as_ref()
            .ok_or(ImportError::WorkspaceUnavailable)?;
        let path = required_string(arguments, "path")?;
        let table = required_string(arguments, "table")?;
        let header = optional_bool(arguments, "header", true)?;
        let delimiter = optional_delimiter(arguments)?;
        let replace = match arguments.get("if_exists") {
            None => false,
            Some(serde_json::Value::String(value)) => match value.as_str() {
                "fail" => false,
                "replace" => true,
                _ => return Err(ImportError::InvalidIfExists.into()),
            },
            Some(_) => return Err(ImportError::InvalidArguments.into()),
        };
        let file = workspace
            .read_for_scratch_import(path)
            .map_err(ImportError::from)?;
        if file.bytes.len() > MAX_IMPORT_FILE_BYTES {
            return Err(ImportError::FileTooLarge {
                max: MAX_IMPORT_FILE_BYTES,
            }
            .into());
        }
        let db = self.db.clone();
        let bytes = file.bytes;
        let table = table.to_owned();
        tokio::task::spawn_blocking(move || {
            import_bytes(&db, &table, &bytes, header, delimiter, replace)
        })
        .await
        .map_err(|_| ScratchError::Import(ImportError::Database))?
        .map_err(ScratchError::Import)
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
            SCRATCH_SQL_TOOL => {
                if arguments.len() != 1 || !arguments.contains_key("sql") {
                    return Err(ToolError::UnsupportedProperty);
                }
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ToolError::SqlNotString)?;
                let result = self.run(sql).await.map_err(scratch_tool_error)?;
                serde_json::to_value(result).map_err(|_| ToolError::QueryResultUnavailable)
            }
            SCRATCH_IMPORT_TOOL => {
                let result = self.import(arguments).await.map_err(scratch_tool_error)?;
                serde_json::to_value(result).map_err(|_| ToolError::QueryResultUnavailable)
            }
            _ => Err(ToolError::UnsupportedTool),
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

fn required_string<'a>(
    arguments: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, ScratchError> {
    arguments
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or(ScratchError::Import(ImportError::InvalidArguments))
}

fn optional_bool(
    arguments: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, ScratchError> {
    match arguments.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or(ScratchError::Import(ImportError::InvalidArguments)),
    }
}

fn optional_delimiter(
    arguments: &serde_json::Map<String, serde_json::Value>,
) -> Result<u8, ScratchError> {
    let value = match arguments.get("delimiter") {
        None => ",",
        Some(serde_json::Value::String(value)) => value,
        Some(_) => return Err(ImportError::InvalidArguments.into()),
    };
    if value.len() == 1 && value.is_ascii() {
        Ok(value.as_bytes()[0])
    } else {
        Err(ImportError::InvalidDelimiter.into())
    }
}
