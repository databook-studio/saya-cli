use crate::proc::OutputSinks;
use saya_types::is_refused_runner_program;

use super::CommandProcessState;
use super::RunProgram;
use super::refuse::{INTERPRETER_REFUSAL, validate_call, validate_interpreter_call};
use super::spawn;
use super::stdin::StdinDelivery;
use super::stdin_channel::StdinInputPipe;
use super::{ProgramOutcome, RunnerError};
use tokio::sync::watch;

pub(crate) struct CommandRunOutcome {
    pub(crate) outcome: ProgramOutcome,
    pub(crate) stdin: Option<StdinDelivery>,
}

impl RunProgram {
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
        let call = self.validated_call(program, argv, timeout_seconds)?;
        spawn::run(
            &self.spawn,
            call,
            // A run_program child pins nothing unless the confinement handed
            // it its child environment. The nested re-entry (`nested_saya`)
            // is the caller that pins run paths.
            &self.pinned_env,
            &self.credentials,
            self.resolver.as_ref(),
            &self.cancellation,
        )
        .await
    }

    pub(crate) async fn run_command(
        &self,
        program: &str,
        argv: &[String],
        timeout_seconds: Option<u64>,
        input: Option<StdinInputPipe>,
        process_state: watch::Sender<CommandProcessState>,
        sinks: Option<OutputSinks>,
    ) -> Result<CommandRunOutcome, RunnerError> {
        let call = self.validated_call(program, argv, timeout_seconds)?;
        let result = spawn::run_command(
            &self.spawn,
            call,
            &self.pinned_env,
            &self.credentials,
            self.resolver.as_ref(),
            &self.cancellation,
            spawn::Channels {
                input,
                process_state: Some(process_state),
                sinks,
            },
        )
        .await?;
        Ok(CommandRunOutcome {
            outcome: result.outcome,
            stdin: result.stdin,
        })
    }

    fn validated_call(
        &self,
        program: &str,
        argv: &[String],
        timeout_seconds: Option<u64>,
    ) -> Result<super::refuse::ValidatedCall, RunnerError> {
        if is_refused_runner_program(program)
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
            )
        } else {
            match &self.runner {
                Some(allowed) => validate_call(
                    allowed,
                    self.spawn.program_dir(),
                    self.default_timeout,
                    timeout_seconds,
                    program,
                    argv,
                ),
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
                    Err(RunnerError::ProgramRefused {
                        program: program.to_owned(),
                        reason,
                    })
                }
            }
        }
    }
}
