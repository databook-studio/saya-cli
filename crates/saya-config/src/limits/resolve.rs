use super::{BudgetLimit, LimitsFile, ResolvedJobLimits, ResolvedRunLimits, database, job, run};
use crate::ConfigError;

/// One answering surface's resolved ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAnsweringLimits {
    pub requests: BudgetLimit,
    pub tool_calls: BudgetLimit,
    pub elapsed_seconds: BudgetLimit,
    pub known_tokens: BudgetLimit,
}

impl Default for ResolvedAnsweringLimits {
    fn default() -> Self {
        Self {
            requests: BudgetLimit::finite(24),
            tool_calls: BudgetLimit::finite(64),
            elapsed_seconds: BudgetLimit::finite(300),
            known_tokens: BudgetLimit::finite(250_000),
        }
    }
}

/// Every surface's resolved ceilings. Each surface keeps its own engine; these
/// are the values it projects into that engine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedLimits {
    /// The interactive session's provider-reported cost ceiling in micro-USD;
    /// `None` is no ceiling. `[limits] session_cost_usd`.
    pub session_cost_micros: Option<u64>,
    /// The database work a turn may spend: `[limits.database]`, or the retired
    /// `[limits.session]` mapped onto it for one release.
    pub database: ResolvedAnsweringLimits,
    /// The one-shot `ask` surface's override of [`Self::database`]: every key
    /// `[limits.ask]` leaves unset keeps the database value.
    pub ask_override: ResolvedAnsweringLimits,
    /// The interactive session's ceilings.
    pub session: ResolvedAnsweringLimits,
    /// The one-shot `ask` surface's ceilings, every unset key already fallen
    /// back to `session`. With nothing declared this equals `session`, which is
    /// what one shared budget meant before `ask` could state its own.
    pub ask: ResolvedAnsweringLimits,
    /// The `saya run` surface's ceilings, projected into the run contract's
    /// `Budgets` by [`ResolvedRunLimits::budgets`].
    pub run: ResolvedRunLimits,
    /// The coding surface's ceilings and its budget mode.
    pub job: ResolvedJobLimits,
    /// One message per retired `[limits.session]`/`[limits.ask]` table this
    /// config states, naming what to write instead. Empty when it states
    /// neither, which is the case an unchanged config is in.
    pub diagnostics: Vec<String>,
}

/// Resolves `[limits]`. The keys are spelled out rather than assembled from the
/// sub-table name so an invalid value names the exact config key to fix.
pub(crate) fn resolve(file: &LimitsFile) -> Result<ResolvedLimits, ConfigError> {
    let defaults = ResolvedAnsweringLimits::default();
    let session = ResolvedAnsweringLimits {
        requests: BudgetLimit::parse(
            file.session.requests.as_ref(),
            "limits.session.requests",
            defaults.requests,
        )?,
        tool_calls: BudgetLimit::parse(
            file.session.tool_calls.as_ref(),
            "limits.session.tool_calls",
            defaults.tool_calls,
        )?,
        elapsed_seconds: BudgetLimit::parse(
            file.session.elapsed_seconds.as_ref(),
            "limits.session.elapsed_seconds",
            defaults.elapsed_seconds,
        )?,
        known_tokens: BudgetLimit::parse(
            file.session.known_tokens.as_ref(),
            "limits.session.known_tokens",
            defaults.known_tokens,
        )?,
    };
    let ask = ResolvedAnsweringLimits {
        requests: BudgetLimit::parse(
            file.ask.requests.as_ref(),
            "limits.ask.requests",
            session.requests,
        )?,
        tool_calls: BudgetLimit::parse(
            file.ask.tool_calls.as_ref(),
            "limits.ask.tool_calls",
            session.tool_calls,
        )?,
        elapsed_seconds: BudgetLimit::parse(
            file.ask.elapsed_seconds.as_ref(),
            "limits.ask.elapsed_seconds",
            session.elapsed_seconds,
        )?,
        known_tokens: BudgetLimit::parse(
            file.ask.known_tokens.as_ref(),
            "limits.ask.known_tokens",
            session.known_tokens,
        )?,
    };
    let resolved_database = database::resolve(file, session)?;
    Ok(ResolvedLimits {
        session_cost_micros: job::cost_micros(
            file.session_cost_usd.as_ref(),
            "limits.session_cost_usd",
        )?,
        database: resolved_database.database,
        ask_override: resolved_database.ask_override,
        session,
        ask,
        run: run::resolve(&file.run)?,
        job: job::resolve(&file.job)?,
        diagnostics: resolved_database.diagnostics,
    })
}
