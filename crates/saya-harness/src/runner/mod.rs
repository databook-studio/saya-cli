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
use saya_types::{InterpreterScope, RunnerScope, is_refused_runner_program};

use refuse::{INTERPRETER_REFUSAL, validate_call, validate_interpreter_call};
use sandbox::RunnerSpawn;

/// The tool's name in the run engine's toolset.
pub const RUN_PROGRAM_TOOL: &str = "run_program";

/// The admitted `run_program` tool: one allowlisted program per call, under
/// the proven sandbox, with the step-narrowed allowlist and the declared
/// credentials.
///
/// The step's scopes open two doors on the same tool, and never one scope
/// wearing the other's name: the **runner door** admits the programs the
/// step's `RunnerScope` names; the **interpreter door** admits only the
/// names the runner refuses by name and the step's `InterpreterScope`
/// explicitly carries — the `--allow interpreter:<program>` grant, which
/// voids the typed-argv contract's behavioural half and is granted by the
/// typed token alone. A refused name outside the interpreter scope falls
/// back to the runner's byte-identical refusal, whatever doors the step
/// holds.
pub struct RunProgram {
    spawn: RunnerSpawn,
    /// The step's narrowed runner programs; `None` when the step asked for
    /// none — every non-refused name is then not allowlisted.
    runner: Option<RunnerScope>,
    /// The step's narrowed interpreter programs; `None` when the step asked
    /// for none — every refused name keeps the runner's byte-identical
    /// refusal.
    interpreters: Option<InterpreterScope>,
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
        Self::for_step(spawn, Some(allowed), None, default_timeout, resolver)
    }

    /// Builds the tool from one step's capabilities: the runner door opens
    /// on the step's `RunnerScope`, the interpreter door on the step's
    /// `InterpreterScope` — each `None` when the step did not ask, so a
    /// step that did not ask never has the door at all.
    pub fn for_step(
        spawn: RunnerSpawn,
        runner: Option<RunnerScope>,
        interpreters: Option<InterpreterScope>,
        default_timeout: Duration,
        resolver: SharedCredentialSource,
    ) -> Self {
        Self {
            record_dir: spawn.fs_roots()[0].join("run_program"),
            spawn,
            runner,
            interpreters,
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

    /// Runs one validated call and returns the child's report. The door a
    /// call goes through is decided by the name: one the runner refuses by
    /// name enters the interpreter door only when the step's interpreter
    /// scope explicitly carries it — every other refused name keeps the
    /// runner's byte-identical refusal — and every other name enters the
    /// runner door, which a step without a runner scope holds shut.
    pub async fn run(
        &self,
        program: &str,
        argv: &[String],
        timeout_seconds: Option<u64>,
    ) -> Result<ProgramOutcome, RunnerError> {
        let call = if is_refused_runner_program(program)
            && self
                .interpreters
                .as_ref()
                .is_some_and(|allowed| allowed.contains(program))
        {
            validate_interpreter_call(
                self.interpreters
                    .as_ref()
                    .expect("the door was just checked"),
                self.spawn.program_dir(),
                self.default_timeout,
                timeout_seconds,
                program,
                argv,
            )?
        } else {
            match &self.runner {
                Some(allowed) => validate_call(
                    allowed,
                    self.spawn.program_dir(),
                    self.default_timeout,
                    timeout_seconds,
                    program,
                    argv,
                )?,
                None => {
                    // The step holds no runner scope: a refused name keeps
                    // the runner's byte-identical refusal, everything else is
                    // simply not allowlisted.
                    let reason = if is_refused_runner_program(program) {
                        INTERPRETER_REFUSAL
                    } else {
                        return Err(RunnerError::ProgramNotAllowlisted {
                            program: program.to_owned(),
                        });
                    };
                    return Err(RunnerError::ProgramRefused {
                        program: program.to_owned(),
                        reason,
                    });
                }
            }
        };
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
