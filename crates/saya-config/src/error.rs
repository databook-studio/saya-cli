use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("job extension profile is invalid or exceeds its bounds")]
    InvalidCodingExtension,
    /// An `[agents]` entry that is not a usable helper type: a name that is not
    /// a bounded lowercase identifier (or is one of the table's own two
    /// settings), a key that is not a table at all, or a declaration that would
    /// widen a type instead of narrowing it. A helper type is a shape, so a
    /// widening entry is refused rather than silently ignored.
    #[error("invalid [agents] entry {name:?}: {reason}")]
    InvalidAgentType { name: String, reason: String },
    /// An `[agents.<name>] model` is copied into the provider request its
    /// helpers send, so it carries exactly the bound an endpoint's own model
    /// carries — the same value, in the same place.
    #[error("setting agents.{name}.model has {value} characters; the limit is {max}")]
    AgentModelTooLong {
        name: String,
        value: usize,
        max: usize,
    },
    #[error("limits.job.budget_mode = \"uncapped\" conflicts with explicitly configured {setting}")]
    JobBudgetModeConflict { setting: &'static str },
    /// A `[limits.job]` dimension written as `"unlimited"`. The durable
    /// budget policy is either finite or uncapped, so a mixed one has no
    /// spelling: the message names the key that does remove every ceiling.
    #[error(
        "setting {key} cannot be `unlimited`: write limits.job.budget_mode = \"uncapped\" \
         to remove every coding ceiling at once"
    )]
    JobLimitUnlimited { key: &'static str },
    /// A `[web_search]` setting that cannot be used as stated. Every case is
    /// a different fix and the reason carries the whole of it, the way
    /// [`ConfigError::InvalidAgentType`] carries its own.
    #[error("invalid [web_search]: {reason}")]
    InvalidWebSearch { reason: &'static str },
    /// A `web_search.provider` that names no backend saya has. Refused rather
    /// than ignored: a provider that resolved to "no search" would leave the
    /// user believing they had configured one.
    #[error(
        "web_search.provider {provider:?} is not a search backend; use \"searxng\" or \"brave\""
    )]
    WebSearchProviderUnknown { provider: String },
    /// An `[mcp_servers]` entry that cannot be started as stated: a name that
    /// is not a usable server name, no `command`, a bound exceeded, or a
    /// remote server, which this release does not support. Every case is a
    /// different fix and the reason carries the whole of it.
    #[error("invalid [mcp_servers]: {reason}")]
    InvalidMcpServers { reason: String },
    #[error("invalid TOML: {0}")]
    Parse(String),
    #[error("config file has {bytes} bytes; maximum is {max_bytes}")]
    ConfigFileTooLarge { bytes: usize, max_bytes: usize },
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
    /// A `[limits]` dimension written as anything but a positive integer or the
    /// literal `unlimited`. `key` is the full dotted config key, so the message
    /// names the exact line to fix.
    #[error("setting {key} must be a positive integer or the literal `unlimited`")]
    InvalidLimit { key: &'static str },
    /// An endpoint's model or base URL is too long to carry safely through
    /// provider construction and diagnostics. The source field is carried so
    /// an inherited `[ai]` value is distinguished from an endpoint override.
    #[error("setting {field} for endpoint {endpoint:?} has {value} characters; the limit is {max}")]
    EndpointStringTooLong {
        field: &'static str,
        endpoint: String,
        value: usize,
        max: usize,
    },
    /// A map-valued setting carries a key that cannot name the thing it
    /// keys — a `[limits.run] tokens_per_endpoint` endpoint name that is
    /// empty, carries whitespace or control characters, or exceeds the name
    /// bound.
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
    /// A `[host_commands]` `pass_env` entry that is not a well-formed
    /// `NAME=value` name. The child's own rule, checked at resolve so the
    /// refusal names the section, not the spawn.
    #[error(
        "setting host_commands.pass_env has an invalid variable name {name:?}: \
             names are ASCII letters, digits, and underscores, not starting with a digit"
    )]
    InvalidHostPassEnv { name: String },
    /// A `[session_commands] deny` entry that is not a bare program name: a
    /// path, traversal, prefix, or glob. Deny bounds the direct ask, so an
    /// entry that reads wider than one name is the prefix fiction in the
    /// comforting direction — strictly weaker than the name deny offered
    /// instead.
    #[error("setting session_commands.deny has an invalid program {program:?}: {reason}")]
    InvalidSessionDeny {
        program: String,
        reason: &'static str,
    },
    /// A project-layer `[session_commands]` section: a model-writable file
    /// must never state the deny list. A model-writable deny could name every
    /// program the session's work needs, herding it off the contained doors
    /// onto the unsandboxed host lane — refusal as escalation. A hard
    /// refusal — `--trust-project-config` does not unlock it.
    #[error(
        "project-layer [session_commands] is refused: the project config is model-writable once \
         write is granted, and a model-writable file must never state the session \
         deny list; state it in your user config or at launch instead"
    )]
    SessionCommandsFromProject,
    /// A project-layer `[host_commands]` section: a model-writable file must
    /// never shape unsandboxed execution — not enable it, not widen its
    /// timeout, not name its env. A hard refusal — the project file is
    /// model-writable once `write` is granted, and
    /// `--trust-project-config` does not unlock it. No deny-list stub rides
    /// along: the deny list is a separate section.
    #[error(
        "project-layer [host_commands] is refused: the project config is model-writable once \
         write is granted, and a model-writable file must never shape unsandboxed \
         execution; state host commands in your user config or at launch instead"
    )]
    HostCommandsFromProject,
    /// A `[job] program_dir` that is a relative path. The canonical form
    /// must not depend on the working directory the config was loaded from —
    /// a relative path would resolve to a different directory per invocation,
    /// and the host's program provisioning is only as real as the one
    /// directory it proved.
    #[error(
        "setting job.program_dir {path:?} must be an absolute path: the canonical form \
         must not depend on the working directory the config was loaded from"
    )]
    RelativeCodingProgramDir { path: String },
    /// A key stated in the wrong one of the two tables whose names differ by
    /// one letter: `[job]`, the job host's own settings, and `[jobs]`, a run's
    /// download budgets. The collision is diagnosed by name rather than left
    /// to serde's `unknown field`, which cannot say which table was meant —
    /// see `model::job_keys`.
    #[error("{message}")]
    MisplacedJobKey { message: String },
    /// A `[tools] allow` list that does not name a set of governed tools: an
    /// empty list, a word that names no tool, or a connection-tied tool. The
    /// inner error carries the whole refusal — including the valid names — so
    /// the user is told what to write instead.
    #[error(transparent)]
    ToolsAllow(#[from] crate::ToolsAllowError),
    /// One `[hooks]` entry that cannot be run: an empty or oversized command,
    /// an argv over its bounds, or a timeout outside its range. Carries the
    /// event and the entry's one-based position, so the fix points at the line
    /// rather than at the table.
    #[error("invalid [hooks] {event} entry #{index}: {reason}")]
    InvalidHook {
        event: &'static str,
        index: usize,
        reason: String,
    },
    /// More hooks for one event than the engine will run. Each entry is a
    /// process on the agent loop's own path, so the count is a bound on the
    /// loop rather than a policy the user widens by writing more entries.
    #[error("setting hooks.{event} lists {value} entries; the limit is {max}")]
    TooManyHooks {
        event: &'static str,
        value: usize,
        max: usize,
    },
}
