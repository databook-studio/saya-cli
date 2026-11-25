//! The one `[limits]` table: every ceiling a surface enforces, in one
//! vocabulary.
//!
//! One resolution and one set of names serve every surface; each surface keeps
//! its own engine and projects these values into it (AGENTS.md, "One
//! operation, multiple adapters"). A dimension is either a positive integer or
//! the literal `unlimited`; a value that is neither is refused at resolve time
//! rather than clamped, so a typo cannot silently become an unbounded run.
//!
//! `[limits.database]` bounds the database work a turn may spend, from its
//! first database call on. The retired `[limits.session]` and `[limits.ask]`
//! tables still resolve for one release — see [`database`] — and their resolved
//! fields stay exposed for the surfaces that read them today. They are
//! protected — see
//! [`crate::layers`]' protected snapshot — because a repository's
//! `.saya/config.toml` is untrusted input and must not raise a ceiling the user
//! set.
//!
//! `[limits.run]` bounds `saya run` and is deliberately *not* protected: it
//! holds the four `[jobs]` ceilings, which a project layer may set, and a
//! run ceiling is a cost control rather than a security boundary.
//!
//! `[limits.job]` bounds a coding job's root loop, its helpers, and the
//! durable budget ledger they share. It is protected whole, like the `[coding]`
//! keys it replaces: a repository's `.saya/config.toml` is untrusted input and
//! must not raise a ceiling the user set. `budget_mode = "uncapped"` is the one
//! unlimited spelling there — see [`job`].

mod database;
mod dimension;
mod job;
mod merge;
mod resolve;
mod run;

pub use dimension::BudgetLimit;
pub use job::ResolvedJobLimits;
pub use resolve::{ResolvedAnsweringLimits, ResolvedLimits};
pub use run::ResolvedRunLimits;

pub(crate) use merge::{merge, merge_job};
pub(crate) use resolve::resolve;

use std::collections::BTreeMap;

use saya_types::CodingBudgetMode;
use serde::Deserialize;

/// The `[limits]` section. Sub-tables arrive surface by surface, and each one
/// carries its own protection decision rather than inheriting the section's.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsFile {
    /// The interactive session's cost ceiling in US dollars, across every
    /// reported request. Absent means no ceiling. Protected like the
    /// answering tables: a project layer must not raise a spend ceiling the
    /// operator set.
    #[serde(default)]
    pub session_cost_usd: Option<toml::Value>,
    /// The interactive session's ceilings, and the fallback for every
    /// `[limits.ask]` key left unset.
    #[serde(default)]
    pub session: AnsweringLimitsFile,
    /// The one-shot `ask` surface's ceilings. Empty means "the session's" for
    /// every dimension.
    #[serde(default)]
    pub ask: AnsweringLimitsFile,
    /// The database work a turn may spend, from the first database call on.
    /// `Some` records that a layer stated the table — an empty table still
    /// means "the defaults", so presence cannot be read off the keys.
    #[serde(default)]
    pub database: Option<AnsweringLimitsFile>,
    /// The `saya run` surface's ceilings. Ordinary, not protected.
    #[serde(default)]
    pub run: RunLimitsFile,
    /// A coding job's ceilings and its budget mode. Protected whole.
    #[serde(default)]
    pub job: JobLimitsFile,
}

/// One answering surface's raw ceilings. Values stay raw TOML until resolution,
/// so a malformed value cannot become unlimited on the way through the layers.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnsweringLimitsFile {
    /// Logical model requests the surface may make.
    pub requests: Option<toml::Value>,
    /// Tool calls the surface may make.
    pub tool_calls: Option<toml::Value>,
    /// Wall-clock seconds the surface may spend answering.
    pub elapsed_seconds: Option<toml::Value>,
    /// Known reported tokens the surface may spend.
    pub known_tokens: Option<toml::Value>,
}

impl AnsweringLimitsFile {
    /// Whether a layer stated this table at all. A table naming no key states
    /// nothing, so it maps onto nothing and needs no diagnostic.
    pub(crate) fn is_stated(&self) -> bool {
        self.requests.is_some()
            || self.tool_calls.is_some()
            || self.elapsed_seconds.is_some()
            || self.known_tokens.is_some()
    }
}

/// The run surface's raw ceilings. Same discipline as the answering tables:
/// values stay raw TOML until resolution, so a malformed value cannot become
/// unlimited on the way through the layers.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunLimitsFile {
    /// Wall-clock seconds a run may spend.
    pub wall_clock_seconds: Option<toml::Value>,
    /// Turns across the run's episodes.
    pub turns: Option<toml::Value>,
    /// Tool calls across the run's episodes.
    pub tool_calls: Option<toml::Value>,
    /// Per-endpoint token ceilings, keyed by run-scoped endpoint name. A map
    /// rather than a dimension: an endpoint left out has no ceiling, so the
    /// `unlimited` spelling has nothing to say here.
    pub tokens_per_endpoint: Option<BTreeMap<String, u64>>,
    /// Consecutive model turns without progress before a step stops. Zero
    /// disables the guard.
    pub no_progress_turns: Option<toml::Value>,
    /// Identical results from one call, in a row, before a step stops. Zero
    /// disables the guard.
    pub repeat_limit: Option<toml::Value>,
}

/// The coding surface's raw ceilings, beside the mode that decides whether any
/// of them apply. Same discipline as the other sub-tables: values stay raw TOML
/// until resolution, so a malformed value cannot become unlimited on the way
/// through the layers.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobLimitsFile {
    /// `bounded` (the default) or `uncapped`. The only unlimited spelling in
    /// this table: the durable budget policy is finite or uncapped and cannot
    /// represent a mixed one.
    pub budget_mode: Option<CodingBudgetMode>,
    /// Logical model requests the root loop may make.
    pub logical_requests: Option<toml::Value>,
    /// Physical transport attempts, which must fund the logical requests.
    pub transport_attempts: Option<toml::Value>,
    /// Tool calls the root loop may make.
    pub tool_calls: Option<toml::Value>,
    /// Known reported tokens one model loop may spend.
    pub known_tokens: Option<toml::Value>,
    /// The optional job-wide ceiling across every model loop; absent (the
    /// default) means no job-wide ceiling.
    pub job_tokens: Option<toml::Value>,
    /// Wall-clock seconds the job's owner deadline allows.
    pub elapsed_seconds: Option<toml::Value>,
    /// Consecutive model turns without progress before the job stops. Zero
    /// disables the guard.
    pub no_progress_turns: Option<toml::Value>,
    /// Identical results from one call, in a row, before the job stops. Zero
    /// disables the guard.
    pub repeat_limit: Option<toml::Value>,
    /// Wall-clock seconds the progress guard allows. Absent means unlimited
    /// when an operator is attached; the job adapter states 7,200 s when none
    /// is.
    pub wall_clock_seconds: Option<toml::Value>,
    /// Provider-reported cost ceiling in US dollars. Absent means no cost
    /// guard.
    pub max_cost_usd: Option<toml::Value>,
}
