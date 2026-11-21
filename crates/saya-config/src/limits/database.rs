//! `[limits.database]`: the work a turn's database calls may spend, plus the
//! one-release mapping of the retired `[limits.session]` and `[limits.ask]`
//! tables onto it.
//!
//! Neither retired table ever bounded a conversation. Since the database
//! plugin took over the question budget they have fed one turn's database work,
//! so a user who wrote them meant to bound that work: their keys are read
//! rather than dropped, because ignoring a stated ceiling would silently widen
//! a bound. Each retired table raises one diagnostic naming its replacement,
//! and the mapping is accepted until the next release.
//!
//! `[limits.ask]` is the exception in scope, not in kind: it always bounded the
//! one-shot `ask` surface, so it maps to that surface's override of the
//! database budget rather than to the budget itself.

use super::{AnsweringLimitsFile, BudgetLimit, LimitsFile, ResolvedAnsweringLimits};
use crate::ConfigError;

/// The dotted keys one answering table's four dimensions are refused under.
#[derive(Clone, Copy)]
struct Keys {
    requests: &'static str,
    tool_calls: &'static str,
    elapsed_seconds: &'static str,
    known_tokens: &'static str,
}

const DATABASE_KEYS: Keys = Keys {
    requests: "limits.database.requests",
    tool_calls: "limits.database.tool_calls",
    elapsed_seconds: "limits.database.elapsed_seconds",
    known_tokens: "limits.database.known_tokens",
};

const ASK_KEYS: Keys = Keys {
    requests: "limits.ask.requests",
    tool_calls: "limits.ask.tool_calls",
    elapsed_seconds: "limits.ask.elapsed_seconds",
    known_tokens: "limits.ask.known_tokens",
};

/// The database budget this config resolves, the `ask` surface's override of
/// it, and one diagnostic per retired table the config still states.
pub(super) struct DatabaseLimits {
    pub database: ResolvedAnsweringLimits,
    pub ask_override: ResolvedAnsweringLimits,
    pub diagnostics: Vec<String>,
}

/// Resolves `[limits.database]` and the two tables that used to stand in for
/// it. `session` is the already-resolved `[limits.session]`: when that retired
/// table is the only one stated, its values become the database budget as they
/// stand, so the mapping cannot disagree with what the old field reports.
pub(super) fn resolve(
    file: &LimitsFile,
    session: ResolvedAnsweringLimits,
) -> Result<DatabaseLimits, ConfigError> {
    let mut diagnostics = Vec::new();
    let defaults = ResolvedAnsweringLimits::default();
    let database = match &file.database {
        Some(table) => {
            if file.session.is_stated() {
                diagnostics.push(ignored("limits.session"));
            }
            answering(table, DATABASE_KEYS, defaults)?
        }
        None if file.session.is_stated() => {
            diagnostics.push(renamed("limits.session"));
            session
        }
        None => defaults,
    };
    let ask_override = answering(&file.ask, ASK_KEYS, database)?;
    if file.ask.is_stated() {
        diagnostics.push(renamed("limits.ask"));
    }
    Ok(DatabaseLimits {
        database,
        ask_override,
        diagnostics,
    })
}

/// Reads one answering table, every unset key keeping `fallback`.
fn answering(
    table: &AnsweringLimitsFile,
    keys: Keys,
    fallback: ResolvedAnsweringLimits,
) -> Result<ResolvedAnsweringLimits, ConfigError> {
    Ok(ResolvedAnsweringLimits {
        requests: BudgetLimit::parse(table.requests.as_ref(), keys.requests, fallback.requests)?,
        tool_calls: BudgetLimit::parse(
            table.tool_calls.as_ref(),
            keys.tool_calls,
            fallback.tool_calls,
        )?,
        elapsed_seconds: BudgetLimit::parse(
            table.elapsed_seconds.as_ref(),
            keys.elapsed_seconds,
            fallback.elapsed_seconds,
        )?,
        known_tokens: BudgetLimit::parse(
            table.known_tokens.as_ref(),
            keys.known_tokens,
            fallback.known_tokens,
        )?,
    })
}

/// The diagnostic a mapped retired table raises: it was renamed, its keys are
/// still honoured this release, and the reader is told what to write instead.
fn renamed(table: &str) -> String {
    format!(
        "[{table}] is now [limits.database]: it bounds database work in a turn, not the \
         session; rename the table (accepted until the next release)"
    )
}

/// The diagnostic a retired table raises beside a stated `[limits.database]`:
/// the new table won, so the old one's keys no longer apply.
fn ignored(table: &str) -> String {
    format!(
        "[{table}] was ignored: [limits.database] is set, and the new table states the \
         database budget; delete [{table}]"
    )
}
