use saya_types::CodingBudgetMode;

use super::dimension::parse_count;
use super::{BudgetLimit, JobLimitsFile};
use crate::ConfigError;
use crate::bounds::require_u64;

/// A coding job's resolved ceilings — the values the root loop, its helpers,
/// and the durable budget ledger are admitted under.
///
/// `budget_mode` is part of this table rather than `[coding]` because it *is*
/// the unlimited spelling: a coding job's durable policy is either every
/// dimension finite or none of them, so there is no per-dimension `unlimited`
/// to state. `Uncapped` therefore resolves every dimension to
/// [`BudgetLimit::Unlimited`]; the accessors apply that rule, so the adapters
/// read a ceiling without asking the mode again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedJobLimits {
    pub budget_mode: CodingBudgetMode,
    /// Logical model requests the root loop may make.
    pub logical_requests: BudgetLimit,
    /// Physical transport attempts, which must fund the logical requests.
    pub transport_attempts: BudgetLimit,
    /// Tool calls the root loop may make.
    pub tool_calls: BudgetLimit,
    /// Known reported tokens one model loop may spend.
    pub known_tokens: BudgetLimit,
    /// The optional job-wide ceiling across every model loop. `Unlimited` (the
    /// default) means no job-wide ceiling.
    pub job_tokens: BudgetLimit,
    /// Wall-clock seconds the job's owner deadline allows.
    pub elapsed_seconds: BudgetLimit,
    /// Consecutive model turns without progress before the job stops. Zero
    /// disables the guard; the job still ends on the repeat guard.
    pub no_progress_turns: u32,
    /// Identical results from one call, in a row, before the job stops. Zero
    /// disables the guard.
    pub repeat_limit: u32,
    /// Wall-clock seconds the progress guard allows. `Unlimited` is the
    /// default and means no wall-clock guard, attended or not; a configured
    /// value is honoured exactly.
    pub wall_clock_seconds: BudgetLimit,
    /// Provider-reported cost ceiling in micro-USD; `None` is no guard.
    pub max_cost_micros: Option<u64>,
}

impl Default for ResolvedJobLimits {
    /// The defaults: runaway backstops far above any healthy job, unlimited
    /// tokens, and the two progress guards. The request and tool backstops
    /// are not spend targets — a job that reaches them has already failed to
    /// make progress and is stopped by the progress guards first.
    fn default() -> Self {
        Self {
            budget_mode: CodingBudgetMode::Bounded,
            logical_requests: BudgetLimit::finite(DEFAULT_LOGICAL_REQUESTS),
            transport_attempts: BudgetLimit::finite(DEFAULT_TRANSPORT_ATTEMPTS),
            tool_calls: BudgetLimit::finite(DEFAULT_TOOL_CALLS),
            known_tokens: BudgetLimit::Unlimited,
            job_tokens: BudgetLimit::Unlimited,
            elapsed_seconds: BudgetLimit::finite(DEFAULT_ELAPSED_SECONDS),
            no_progress_turns: DEFAULT_NO_PROGRESS_TURNS,
            repeat_limit: DEFAULT_REPEAT_LIMIT,
            wall_clock_seconds: BudgetLimit::Unlimited,
            max_cost_micros: None,
        }
    }
}

impl ResolvedJobLimits {
    /// `budget_mode = "uncapped"`: no ceiling on any dimension.
    fn uncapped() -> Self {
        Self {
            budget_mode: CodingBudgetMode::Uncapped,
            logical_requests: BudgetLimit::Unlimited,
            transport_attempts: BudgetLimit::Unlimited,
            tool_calls: BudgetLimit::Unlimited,
            known_tokens: BudgetLimit::Unlimited,
            job_tokens: BudgetLimit::Unlimited,
            elapsed_seconds: BudgetLimit::Unlimited,
            no_progress_turns: DEFAULT_NO_PROGRESS_TURNS,
            repeat_limit: DEFAULT_REPEAT_LIMIT,
            wall_clock_seconds: BudgetLimit::Unlimited,
            max_cost_micros: None,
        }
    }

    /// The ceiling in force on one dimension. `uncapped` states "no ceiling at
    /// all", so a dimension that still holds a number is not a ceiling either:
    /// the mode decides, whatever the field says. Every adapter reads the
    /// ceilings through these accessors, so no read site has to ask the mode
    /// again.
    fn in_force(&self, dimension: BudgetLimit) -> BudgetLimit {
        match self.budget_mode {
            CodingBudgetMode::Uncapped => BudgetLimit::Unlimited,
            // Any mode that is not the unlimited spelling keeps the dimension
            // it states, so an unknown mode stays bounded rather than silently
            // dropping every ceiling.
            _ => dimension,
        }
    }

    pub fn logical_requests(&self) -> BudgetLimit {
        self.in_force(self.logical_requests)
    }

    pub fn transport_attempts(&self) -> BudgetLimit {
        self.in_force(self.transport_attempts)
    }

    pub fn tool_calls(&self) -> BudgetLimit {
        self.in_force(self.tool_calls)
    }

    pub fn known_tokens(&self) -> BudgetLimit {
        self.in_force(self.known_tokens)
    }

    pub fn job_tokens(&self) -> BudgetLimit {
        self.in_force(self.job_tokens)
    }

    pub fn elapsed_seconds(&self) -> BudgetLimit {
        self.in_force(self.elapsed_seconds)
    }
}

/// Resolves `[limits.job]`. Each key is spelled out rather than assembled
/// from the sub-table name so an invalid value names the exact config key to
/// fix.
pub(crate) fn resolve(file: &JobLimitsFile) -> Result<ResolvedJobLimits, ConfigError> {
    let budget_mode = file.budget_mode.unwrap_or(CodingBudgetMode::Bounded);
    let defaults = ResolvedJobLimits::default();
    let no_progress_turns = parse_count(
        file.no_progress_turns.as_ref(),
        "limits.job.no_progress_turns",
        defaults.no_progress_turns,
        MAX_NO_PROGRESS_TURNS,
    )?;
    let repeat_limit = parse_count(
        file.repeat_limit.as_ref(),
        "limits.job.repeat_limit",
        defaults.repeat_limit,
        MAX_REPEAT_LIMIT,
    )?;
    let wall_clock_seconds = wall_clock(
        file.wall_clock_seconds.as_ref(),
        "limits.job.wall_clock_seconds",
        defaults.wall_clock_seconds,
    )?;
    let max_cost_micros = cost_micros(file.max_cost_usd.as_ref(), "limits.job.max_cost_usd")?;
    if budget_mode == CodingBudgetMode::Uncapped {
        reject_stated_dimensions(file)?;
        return Ok(ResolvedJobLimits {
            no_progress_turns,
            repeat_limit,
            wall_clock_seconds,
            max_cost_micros,
            ..ResolvedJobLimits::uncapped()
        });
    }
    let logical_requests = finite_dimension(
        file.logical_requests.as_ref(),
        "limits.job.logical_requests",
        defaults.logical_requests,
        MAX_LOGICAL_REQUESTS,
    )?;
    let transport_attempts = finite_dimension(
        file.transport_attempts.as_ref(),
        "limits.job.transport_attempts",
        defaults.transport_attempts,
        MAX_TRANSPORT_ATTEMPTS,
    )?;
    // Transport attempts fund the logical requests: a ceiling below them would
    // refuse work the job was already admitted to do.
    if let (Some(requests), Some(attempts)) = (logical_requests.value(), transport_attempts.value())
        && attempts < requests
    {
        return Err(ConfigError::SettingOutOfRange {
            field: "limits.job.transport_attempts",
            value: attempts as usize,
            min: requests as usize,
            max: MAX_TRANSPORT_ATTEMPTS as usize,
        });
    }
    Ok(ResolvedJobLimits {
        budget_mode,
        logical_requests,
        transport_attempts,
        tool_calls: finite_dimension(
            file.tool_calls.as_ref(),
            "limits.job.tool_calls",
            defaults.tool_calls,
            MAX_TOOL_CALLS,
        )?,
        known_tokens: finite_dimension(
            file.known_tokens.as_ref(),
            "limits.job.known_tokens",
            defaults.known_tokens,
            MAX_KNOWN_TOKENS,
        )?,
        job_tokens: finite_dimension(
            file.job_tokens.as_ref(),
            "limits.job.job_tokens",
            defaults.job_tokens,
            MAX_JOB_TOKENS,
        )?,
        elapsed_seconds: finite_dimension(
            file.elapsed_seconds.as_ref(),
            "limits.job.elapsed_seconds",
            defaults.elapsed_seconds,
            MAX_ELAPSED_SECONDS,
        )?,
        no_progress_turns,
        repeat_limit,
        wall_clock_seconds,
        max_cost_micros,
    })
}

/// One coding dimension: a positive integer within its ceiling. An explicit
/// `"unlimited"` is refused here — the harness's durable budget policy is
/// either finite (V1) or uncapped (V2) and cannot represent a mixed one — so
/// the refusal names `budget_mode`, the spelling that does remove every
/// ceiling at once. An absent key takes the fallback, which is `Unlimited` for
/// `job_tokens` alone.
fn finite_dimension(
    value: Option<&toml::Value>,
    key: &'static str,
    fallback: BudgetLimit,
    max: u64,
) -> Result<BudgetLimit, ConfigError> {
    let Some(value) = value else {
        return Ok(fallback);
    };
    let BudgetLimit::Finite(finite) = BudgetLimit::parse(Some(value), key, fallback)? else {
        return Err(ConfigError::JobLimitUnlimited { key });
    };
    require_u64(finite.get(), key, max)?;
    Ok(BudgetLimit::Finite(finite))
}

/// The wall-clock guard: a positive integer, `unlimited`, or absent. Unlike
/// the durable dimensions, `unlimited` is a real spelling here because this
/// guard is not part of the ledger's finite-or-uncapped policy.
fn wall_clock(
    value: Option<&toml::Value>,
    key: &'static str,
    fallback: BudgetLimit,
) -> Result<BudgetLimit, ConfigError> {
    let limit = BudgetLimit::parse(value, key, fallback)?;
    if let Some(seconds) = limit.value() {
        require_u64(seconds, key, MAX_WALL_CLOCK_SECONDS)?;
    }
    Ok(limit)
}

/// The cost guard in micro-USD. A TOML float or integer is US dollars; absent
/// means no guard. A non-number, a non-positive value, or a value above the
/// stated ceiling is refused naming the key.
fn cost_micros(value: Option<&toml::Value>, key: &'static str) -> Result<Option<u64>, ConfigError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let usd = match value {
        toml::Value::Float(usd) => *usd,
        toml::Value::Integer(usd) => *usd as f64,
        _ => return Err(ConfigError::InvalidLimit { key }),
    };
    if !usd.is_finite() || usd <= 0.0 {
        return Err(ConfigError::InvalidLimit { key });
    }
    if usd > MAX_COST_USD {
        return Err(ConfigError::SettingOutOfRange {
            field: key,
            value: usd as usize,
            min: 1,
            max: MAX_COST_USD as usize,
        });
    }
    Ok(Some((usd * 1_000_000.0).round() as u64))
}

/// `uncapped` states "no ceiling at all", so a dimension beside it is a
/// contradiction rather than an override.
fn reject_stated_dimensions(file: &JobLimitsFile) -> Result<(), ConfigError> {
    for (setting, configured) in [
        (
            "limits.job.logical_requests",
            file.logical_requests.is_some(),
        ),
        (
            "limits.job.transport_attempts",
            file.transport_attempts.is_some(),
        ),
        ("limits.job.tool_calls", file.tool_calls.is_some()),
        ("limits.job.known_tokens", file.known_tokens.is_some()),
        ("limits.job.job_tokens", file.job_tokens.is_some()),
        ("limits.job.elapsed_seconds", file.elapsed_seconds.is_some()),
    ] {
        if configured {
            return Err(ConfigError::JobBudgetModeConflict { setting });
        }
    }
    Ok(())
}

/// The runaway backstops. These bound a loop that has lost its way; a healthy
/// job stops on a progress guard long before them.
const DEFAULT_LOGICAL_REQUESTS: u64 = 2_000;
const DEFAULT_TRANSPORT_ATTEMPTS: u64 = 6_000;
const DEFAULT_TOOL_CALLS: u64 = 10_000;
/// The durable policy needs a finite deadline. Seven days is the runaway
/// backstop; the progress guards, not a wall clock, stop a stuck job.
const DEFAULT_ELAPSED_SECONDS: u64 = 604_800;

/// The sane ceiling per dimension. Every one is above the value an earlier
/// 0.5.0 build used as its default, so a config that states the old ceiling
/// still resolves to exactly what it asked for.
const MAX_LOGICAL_REQUESTS: u64 = 100_000;
const MAX_TRANSPORT_ATTEMPTS: u64 = 300_000;
const MAX_TOOL_CALLS: u64 = 1_000_000;
const MAX_KNOWN_TOKENS: u64 = 1_000_000_000;
const MAX_JOB_TOKENS: u64 = 100_000_000;
const MAX_ELAPSED_SECONDS: u64 = 2_592_000;

/// The progress guards' built-in defaults. Zero disables a guard, so these are
/// stated separately from the positive-integer dimensions above.
const DEFAULT_NO_PROGRESS_TURNS: u32 = 8;
const DEFAULT_REPEAT_LIMIT: u32 = 3;
const MAX_NO_PROGRESS_TURNS: u32 = 10_000;
const MAX_REPEAT_LIMIT: u32 = 10_000;
const MAX_WALL_CLOCK_SECONDS: u64 = 2_592_000;
/// One million US dollars. The micro-USD conversion stays well inside `u64`.
const MAX_COST_USD: f64 = 1_000_000.0;
