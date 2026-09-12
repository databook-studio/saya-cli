use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid TOML: {0}")]
    Parse(String),
    #[error("invalid environment value for {name}: {reason}")]
    InvalidEnvironment { name: String, reason: String },
    #[error("no connection profile was selected")]
    MissingProfile,
    #[error("connection profile {0:?} was not found")]
    UnknownProfile(String),
    #[error("database environment variable {name} is required")]
    MissingDatabaseField { name: &'static str },
    #[error("database type {0:?} is not supported in environment configuration")]
    UnsupportedDatabaseType(String),
    #[error("secret reference {0} could not be resolved")]
    MissingSecret(String),
    #[error("keyring secret references are unavailable in this runtime")]
    KeyringUnavailable,
    #[error("could not read secret file: {0}")]
    SecretFile(String),
    #[error("memory setting {field} = {value} must be within {min}..={max}")]
    MemoryRange {
        field: &'static str,
        value: u32,
        min: u32,
        max: u32,
    },
    /// A non-memory numeric setting is below its floor. Sibling to
    /// [`ConfigError::MemoryRange`] for settings that are not `[memory]` (e.g.
    /// `[ai] context_byte_budget`), which have a minimum but no useful ceiling
    /// — reporting one would mean printing `usize::MAX` at the user.
    #[error("setting {field} = {value} must be at least {min}")]
    SettingBelowMinimum {
        field: &'static str,
        value: usize,
        min: usize,
    },
    /// A non-memory setting exceeds its ceiling. Sibling to
    /// [`ConfigError::SettingBelowMinimum`] for list-shaped settings such as
    /// `[ai] retry_delays_ms`, where the floor is meaningful (an empty list is
    /// a valid "do not retry" choice, so zero is allowed) but a runaway length
    /// is not. `value` is the supplied length and `max` the permitted count.
    #[error("setting {field} lists {value} entries; the limit is {max}")]
    SettingAboveMaximum {
        field: &'static str,
        value: usize,
        max: usize,
    },
    /// A non-memory setting must fall inside an inclusive range with both a
    /// meaningful floor and ceiling (e.g. `[run] candidates`). Sibling to
    /// [`ConfigError::SettingBelowMinimum`] and [`ConfigError::SettingAboveMaximum`]
    /// for settings where zero is meaningless and an unbounded value would be
    /// unsafe — so the accepted range, not just one bound, is reported.
    #[error("setting {field} = {value} must be within {min}..={max}")]
    SettingOutOfRange {
        field: &'static str,
        value: usize,
        min: usize,
        max: usize,
    },
    /// A map-valued setting carries a key that cannot name the thing it
    /// keys — a `[jobs] tokens_per_endpoint` endpoint name that is empty,
    /// carries whitespace or control characters, or exceeds the name bound.
    /// That shape is what the run contracts reject at plan-validation time,
    /// so it is caught at resolve time instead, far from the config mistake.
    /// The key is carried because the field name alone cannot point at the
    /// offender inside a map.
    #[error("setting {field} has an invalid endpoint name {key:?}")]
    InvalidEndpointName { field: &'static str, key: String },
    /// Two `[[ai.endpoints]]` entries in one configuration layer carry the
    /// same name. Endpoints are keyed by name at resolution — a run binds a
    /// role to a name — so a duplicate would make "which endpoint serves this
    /// role" ambiguous. Rejected rather than resolved last-wins.
    #[error("endpoint {0:?} is declared more than once in [ai.endpoints]; remove the duplicate")]
    DuplicateEndpointName(String),
    /// A `[jobs.runner] allow` entry is not a program the runner can honour:
    /// a bare, non-repeating name in the run-scoped shape that is not a shell
    /// or interpreter. The reason is carried because "why not" is the whole
    /// diagnostic — the shapes refused here are refused by the runner tool
    /// too, so approving one would approve a capability that cannot exist.
    #[error("setting {field} has an invalid runner program {program:?}: {reason}")]
    InvalidRunnerProgram {
        field: &'static str,
        program: String,
        reason: &'static str,
    },
    /// A `[jobs.runner] program_dir` that is a relative path. The canonical
    /// form must not depend on the working directory the config was loaded
    /// from — a relative path would resolve to a different directory per
    /// invocation, and the run's probe verdict is only as real as the one
    /// directory it proved.
    #[error(
        "setting runner.program_dir {path:?} must be an absolute path: the canonical form \
         must not depend on the working directory the config was loaded from"
    )]
    RelativeRunnerProgramDir { path: String },
    /// `[jobs.runner] allow` names programs while `program_dir` is
    /// undeclared. The runner resolves every allowlisted program inside one
    /// directory and nowhere else, so an allowlist without its directory
    /// approves programs that cannot run — the same typed resolve error
    /// every other `[jobs]` mistake gets, never a silently-approved
    /// capability that gates nothing.
    #[error(
        "[jobs.runner] allow names programs but runner.program_dir is undeclared: stage \
         the allowlisted programs in one directory and set program_dir to its absolute path"
    )]
    RunnerAllowWithoutProgramDir,
}
