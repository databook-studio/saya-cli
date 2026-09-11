//! The `run_program` tool (M5-4): one allowlisted program, typed argv, no
//! shell, no interpolation — ever.
//!
//! [`RunProgram`] is the admitted tool, constructed around a proven
//! [`RunnerSpawn`] — the M5-3 startup probe's verdict, consumed here, not
//! re-derived — so an unproven host simply never gets this tool. The
//! allowlist is **per step**: the tool carries the plan-narrowed
//! [`RunnerScope`], a subset of the run's approved programs; approving the
//! plan approves the programs it names, and anything else is a typed
//! refusal in `refuse::validate` before anything is spawned. The child's
//! cwd is pinned to the run workspace by the spawn configuration; the
//! environment is built in `env`, empty by default, with only the declared
//! credentials injected. Timeout and cancellation kill the process group
//! (`spawn`); stdout and stderr are ring-buffered with byte caps and
//! redacted unconditionally — the resolved credential values themselves
//! scrubbed by the capture's value registry, then the `redact()` pattern
//! pass — before anything reaches the model or the run's disk record
//! (`output`).

pub mod env;
pub mod error;
pub mod nested_saya;
pub mod output;
pub mod refuse;
pub mod sandbox;
pub(crate) mod spawn;

pub use env::{Credential, CredentialSource, SharedCredentialSource, StaticCredentialSource};
pub use error::RunnerError;
pub use output::{OUTPUT_CAP_BYTES, OutputRing, ProgramOutcome, StreamCapture};

use std::{io, path::PathBuf, time::Duration};

use async_trait::async_trait;
use saya_agent::{
    CancellationToken, LocalStateEffect, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
};
use saya_types::RunnerScope;

use refuse::validate_call;
use sandbox::RunnerSpawn;

/// The tool's name in the run engine's toolset.
pub const RUN_PROGRAM_TOOL: &str = "run_program";

/// The admitted `run_program` tool: one allowlisted program per call, under
/// the proven sandbox, with the step-narrowed allowlist and the declared
/// credentials.
pub struct RunProgram {
    spawn: RunnerSpawn,
    allowed: RunnerScope,
    credentials: Vec<Credential>,
    resolver: SharedCredentialSource,
    default_timeout: Duration,
    cancellation: CancellationToken,
    record_dir: PathBuf,
}

impl RunProgram {
    /// Builds the tool over a proven spawn, the step-narrowed allowlist, the
    /// configured default timeout, and the credential seam.
    pub fn new(
        spawn: RunnerSpawn,
        allowed: RunnerScope,
        default_timeout: Duration,
        resolver: SharedCredentialSource,
    ) -> Self {
        Self {
            record_dir: spawn.fs_roots()[0].join("run_program"),
            spawn,
            allowed,
            credentials: Vec::new(),
            resolver,
            default_timeout,
            cancellation: CancellationToken::new(),
        }
    }

    /// Attaches the credentials the approved plan declared for this step:
    /// only what is on this list can ever reach a child's environment.
    pub fn with_credentials(mut self, credentials: Vec<Credential>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Attaches the run's cancellation token: a cancelled run kills the
    /// child's whole process group and reports the cancellation.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Runs one validated call and returns the child's report.
    pub async fn run(
        &self,
        program: &str,
        argv: &[String],
        timeout_seconds: Option<u64>,
    ) -> Result<ProgramOutcome, RunnerError> {
        let call = validate_call(
            &self.allowed,
            self.spawn.program_dir(),
            self.default_timeout,
            timeout_seconds,
            program,
            argv,
        )?;
        spawn::run(
            &self.spawn,
            call,
            // A run_program child pins nothing: its environment is the
            // declared credentials and nothing else. The nested re-entry
            // (`nested_saya`) is the caller that pins run paths.
            &[],
            &self.credentials,
            self.resolver.as_ref(),
            &self.cancellation,
        )
        .await
    }

    /// The tool definition.
    pub fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: RUN_PROGRAM_TOOL.to_string(),
            description: "Run one allowlisted program with typed argv. Every argument is \
                passed verbatim as one argv element — no shell, no interpolation, no \
                command-line string anywhere. The allowlist is this step's narrowed set; \
                bash, sh, wrappers, and paths are refused. The child runs sandboxed in \
                this run's workspace; output is capped and redacted; a timeout kills the \
                whole process group."
                .into(),
            read_only: false,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "program": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "timeout_seconds": { "type": "integer", "minimum": 1 }
                },
                "required": ["program"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: !self.spawn.net_allow().is_empty(),
                // Plan-gated, like scratch: approving the plan approves the
                // programs it names. The child writes inside the run
                // workspace's fs roots.
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            completion: Some("program ran".into()),
        }
    }

    /// Persists the redacted outcome into the run workspace — the disk
    /// record the model's copy mirrors. Nothing unredacted exists to write.
    fn record(&self, outcome: &ProgramOutcome) -> Result<String, RunnerError> {
        output::record_outcome(&self.record_dir, outcome)
    }
}

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
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| not_typed("every element of args must be a string"))
                })
                .collect::<Result<_, _>>()?,
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

fn not_typed(detail: &'static str) -> RunnerError {
    RunnerError::ArgsNotTyped { detail }
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
