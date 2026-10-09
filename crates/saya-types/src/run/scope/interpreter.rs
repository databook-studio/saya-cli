//! The interpreter capability: the deliberate exception to the runner's
//! shell refusal, granted only by the `--allow interpreter:<program>` token.

use serde::{Deserialize, Serialize};

use super::naming::is_bare_name;
use super::runner::{MAX_RUNNER_PROGRAMS, is_refused_runner_program};
use crate::run::RunContractError;

/// The interpreters a run's approved interpreter scope may name. This family
/// exists beside the runner's, never instead of it: its members are exactly
/// [`is_refused_runner_program`]'s list — the names the runner refuses
/// structurally — and the constructor enforces that mirror, so a name the
/// runner does not refuse can never ride the interpreter family. Approving
/// one is a deliberate, explicit act (the `--allow interpreter:<program>`
/// token); it voids the typed-argv contract's behavioural half, which is why
/// nothing but the typed token grants it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InterpreterScope {
    pub programs: Vec<String>,
}

impl InterpreterScope {
    pub fn new(programs: Vec<String>) -> Result<Self, RunContractError> {
        if programs.is_empty() {
            return Err(RunContractError::EmptyInterpreterPrograms);
        }
        if programs.len() > MAX_RUNNER_PROGRAMS {
            return Err(RunContractError::TooManyInterpreterPrograms);
        }
        for program in &programs {
            // A scope names programs, never paths — the same rule the
            // runner scope carries, shared so the two cannot drift.
            if !is_bare_name(program) {
                return Err(RunContractError::InvalidInterpreterProgram);
            }
            // The family IS the refusal list, mirrored here so a hand-built
            // scope cannot smuggle a non-refused name in: the interpreter
            // door exists only for names the runner refuses, and every other
            // program belongs to the runner family.
            if !is_refused_runner_program(program) {
                return Err(RunContractError::InterpreterProgramNotRefused);
            }
        }
        Ok(Self { programs })
    }

    /// True when `program` is one of this scope's members.
    pub fn contains(&self, program: &str) -> bool {
        self.programs.iter().any(|allowed| allowed == program)
    }
}
