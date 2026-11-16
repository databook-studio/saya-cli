use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFetchJobs {
    /// Per-file download ceiling, in bytes.
    pub max_file_bytes: u64,
    /// Ceiling on total download bytes across the whole run.
    pub max_run_bytes: u64,
    /// Wall-clock budget for one download request, in seconds.
    pub timeout_seconds: u64,
}

impl Default for ResolvedFetchJobs {
    /// The conservative defaults — the same numbers an absent
    /// `[jobs.fetch]` resolves to.
    fn default() -> Self {
        Self {
            max_file_bytes: FETCH_MAX_FILE_BYTES,
            max_run_bytes: FETCH_MAX_RUN_BYTES,
            timeout_seconds: FETCH_TIMEOUT_SECONDS,
        }
    }
}

/// The `[jobs]` section resolved. The run ceilings live in `[limits.run]`
/// (`ResolvedRunLimits`); what is left is the download budget, which is a
/// budget with concrete defaults rather than a ceiling a run declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedJobs {
    /// Download budget defaults for `http_download`. Always
    /// concrete: a run with nothing declared is bounded by the conservative
    /// defaults.
    pub fetch: ResolvedFetchJobs,
}

/// The resolved session deny list, from user-layer `[session_commands]
/// deny`. Refusal-only: bare program names, sorted and deduped, that every
/// session door refuses before grant, prompt, and bypass. Empty when nothing
/// is denied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedSessionDeny {
    /// The denied programs, sorted, deduped.
    pub programs: Vec<String>,
}

/// Resolves `[session_commands] deny`: every entry is a bare name — never a
/// path, traversal, prefix, or glob — refused at resolve time with a typed
/// error. Sorted and deduped, so the journal's start event and the status
/// line carry one canonical order.
pub(crate) fn resolve_session_deny(
    file: crate::model::SessionCommandsFile,
) -> Result<ResolvedSessionDeny, ConfigError> {
    let mut programs: Vec<String> = file.deny;
    for program in &programs {
        if !is_bare_name(program) {
            return Err(ConfigError::InvalidSessionDeny {
                program: program.clone(),
                reason: "a deny entry is a bare program name, never a path, traversal, \
                     prefix, or glob",
            });
        }
    }
    programs.sort();
    programs.dedup();
    Ok(ResolvedSessionDeny { programs })
}

/// Effective host-command defaults, resolved from `[host_commands]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHostCommands {
    /// Parent variables the built child environment carries, by name.
    pub pass_env: Vec<String>,
    /// Per-call ceiling in seconds; a call may narrow it, never widen it.
    pub timeout_seconds: u64,
}

impl Default for ResolvedHostCommands {
    fn default() -> Self {
        Self {
            pass_env: Vec::new(),
            timeout_seconds: HOST_COMMANDS_TIMEOUT_SECONDS,
        }
    }
}
