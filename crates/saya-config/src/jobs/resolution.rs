use super::*;

/// Resolves `[jobs]`: every ceiling is opt-in, and a ceiling left unset is
/// unlimited at the contract level — the engine layers RunSpec and step
/// budgets over these resolved defaults per dimension. Numeric bounds are
/// checked here, at resolve time, with typed errors — never silently
/// clamped at the point of use.
pub(crate) fn resolve(file: &JobsFile) -> Result<ResolvedJobs, ConfigError> {
    if let Some(seconds) = file.wall_clock_seconds {
        require_at_least_one("wall_clock_seconds", seconds)?;
    }
    let tokens_per_endpoint = match &file.tokens_per_endpoint {
        Some(map) => resolve_token_map(map)?,
        None => BTreeMap::new(),
    };
    if let Some(turns) = file.turns {
        require_at_least_one("turns", turns)?;
    }
    if let Some(tool_calls) = file.tool_calls {
        require_at_least_one("tool_calls", tool_calls)?;
    }
    let fetch = resolve_fetch(file.fetch.as_ref())?;
    let runner = resolve_runner(file.runner.as_ref())?;
    let interpreter = resolve_interpreter(file.interpreter.as_ref(), runner.program_dir.as_ref())?;
    Ok(ResolvedJobs {
        wall_clock_seconds: file.wall_clock_seconds,
        tokens_per_endpoint,
        turns: file.turns,
        tool_calls: file.tool_calls,
        fetch,
        runner,
        interpreter,
    })
}

/// Resolves `[jobs.fetch]`: each declared key is checked against the
/// minimum discipline at resolve time, with typed errors — never silently
/// clamped at the point of use. Every key is optional; the defaults are the
/// provisional conservative numbers documented above.
fn resolve_fetch(fetch: Option<&FetchJobsFile>) -> Result<ResolvedFetchJobs, ConfigError> {
    let fetch = fetch.cloned().unwrap_or_default();
    let max_file_bytes = fetch.max_file_bytes.unwrap_or(FETCH_MAX_FILE_BYTES);
    require_at_least_one("fetch.max_file_bytes", max_file_bytes)?;
    let max_run_bytes = fetch.max_run_bytes.unwrap_or(FETCH_MAX_RUN_BYTES);
    require_at_least_one("fetch.max_run_bytes", max_run_bytes)?;
    let timeout_seconds = fetch.timeout_seconds.unwrap_or(FETCH_TIMEOUT_SECONDS);
    require_at_least_one("fetch.timeout_seconds", timeout_seconds)?;
    Ok(ResolvedFetchJobs {
        max_file_bytes,
        max_run_bytes,
        timeout_seconds,
    })
}

/// Resolves `[jobs.runner]`: each declared key is checked at resolve time
/// with typed errors — never silently clamped at the point of use. Program
/// names must have the run-scoped name shape, must not repeat, must stay
/// within the contract's program-count bound, and must never name a shell or
/// interpreter — the runner refuses those structurally, so an allowlist that
/// carried one would approve a capability that cannot exist. The program
/// directory must be absolute (the canonical form must not depend on the
/// working directory the config was loaded from), and an `allow` that names
/// programs requires it.
fn resolve_runner(runner: Option<&RunnerJobsFile>) -> Result<ResolvedRunnerJobs, ConfigError> {
    let runner = runner.cloned().unwrap_or_default();
    let program_dir = match runner.program_dir {
        Some(dir) if !dir.is_absolute() => {
            return Err(ConfigError::RelativeRunnerProgramDir {
                path: dir.display().to_string(),
            });
        }
        other => other,
    };
    let mut allow = Vec::new();
    if let Some(programs) = runner.allow {
        if programs.len() > MAX_RUNNER_PROGRAMS {
            return Err(ConfigError::SettingAboveMaximum {
                field: "runner.allow",
                value: programs.len(),
                max: MAX_RUNNER_PROGRAMS,
            });
        }
        for program in programs {
            if !is_bare_name(&program) {
                return Err(ConfigError::InvalidRunnerProgram {
                    field: "runner.allow",
                    program: program.clone(),
                    reason: "a program entry is a bare name, never a path or traversal",
                });
            }
            if is_refused_runner_program(&program) {
                return Err(ConfigError::InvalidRunnerProgram {
                    field: "runner.allow",
                    program: program.clone(),
                    reason: "shells and interpreters are refused: the runner runs one \
                         allowlisted program with typed argv, and an interpreter would \
                         spawn arbitrary children from inside the allowlist",
                });
            }
            if allow.contains(&program) {
                return Err(ConfigError::InvalidRunnerProgram {
                    field: "runner.allow",
                    program: program.clone(),
                    reason: "declared more than once",
                });
            }
            allow.push(program);
        }
    }
    if !allow.is_empty() && program_dir.is_none() {
        return Err(ConfigError::RunnerAllowWithoutProgramDir);
    }
    let timeout_seconds = runner.timeout_seconds.unwrap_or(RUNNER_TIMEOUT_SECONDS);
    require_at_least_one("runner.timeout_seconds", timeout_seconds)?;
    Ok(ResolvedRunnerJobs {
        allow,
        program_dir,
        timeout_seconds,
    })
}

/// Resolves `[jobs.interpreter]`, mirroring `resolve_runner` exactly: each
/// declared key is checked at resolve time with typed errors — never
/// silently clamped at the point of use. Program names must have the
/// run-scoped name shape, must not repeat, must stay within the contract's
/// program-count bound, and must BE a shell or interpreter the runner
/// refuses — the family is the refusal list, so a member the runner would
/// run is a typed resolve error pointing at `[jobs.runner] allow`, and the
/// two universes stay disjoint by construction. An `allow` naming
/// interpreters requires `[jobs.runner] program_dir`: the interpreters are
/// staged in that one directory, and staging input is trusted-layer
/// business.
fn resolve_interpreter(
    interpreter: Option<&InterpreterJobsFile>,
    program_dir: Option<&std::path::PathBuf>,
) -> Result<ResolvedInterpreterJobs, ConfigError> {
    let interpreter = interpreter.cloned().unwrap_or_default();
    let mut allow = Vec::new();
    if let Some(programs) = interpreter.allow {
        if programs.len() > MAX_RUNNER_PROGRAMS {
            return Err(ConfigError::SettingAboveMaximum {
                field: "interpreter.allow",
                value: programs.len(),
                max: MAX_RUNNER_PROGRAMS,
            });
        }
        for program in programs {
            if !is_bare_name(&program) {
                return Err(ConfigError::InvalidInterpreterProgram {
                    field: "interpreter.allow",
                    program: program.clone(),
                    reason: "an interpreter entry is a bare name, never a path or traversal",
                });
            }
            if !is_refused_runner_program(&program) {
                return Err(ConfigError::InvalidInterpreterProgram {
                    field: "interpreter.allow",
                    program: program.clone(),
                    reason: "not a shell or interpreter the runner refuses — the interpreter \
                         family is the refusal list; declare programs the runner can run \
                         in [jobs.runner] allow",
                });
            }
            if allow.contains(&program) {
                return Err(ConfigError::InvalidInterpreterProgram {
                    field: "interpreter.allow",
                    program: program.clone(),
                    reason: "declared more than once",
                });
            }
            allow.push(program);
        }
    }
    if !allow.is_empty() && program_dir.is_none() {
        return Err(ConfigError::InterpreterAllowWithoutProgramDir);
    }
    Ok(ResolvedInterpreterJobs { allow })
}
