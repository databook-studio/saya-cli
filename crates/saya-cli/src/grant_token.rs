//! The grant token suggestion: the bridge from one tool call to the
//! narrowest `--allow` grammar token a session grant for it would record.
//!
//! The suggestion is the only place that names what an interactive session
//! grant could be, and every token it produces is a word the existing
//! `/allow` grammar accepts — the parser (crate::commands::run::scopes) is
//! the grammar's authority, never a duplicate. `None` is a deliberate
//! answer, not a gap: it means the tool keeps asking every call.
//!
//! A token is suggested only when the surface's composition carries what it
//! names (`facts`, the same `ApprovalFacts` the prompts state): an offer of
//! a capability the composition cannot honour would record a grant that
//! pre-answers asks into refusals — worse than a dead offer, it silences
//! the asks that would say something is wrong (U8). A composition that
//! carries nothing of a family offers nothing from it: the prompt falls
//! back to its two answers, exactly as it does when no token exists.
//!
//! The host spelling for `fetch:` is the run engine's: the engine's fetch
//! policy compares `FetchDestination::new(url.scheme(), url.host_str())`,
//! and the shape is judged by the grammar's own `Destination` rule, so a
//! granted token names the destination the engine would fetch.

mod family;
mod primary;
mod token;
mod wording;

pub(crate) use family::SQL_FAMILY;
pub(crate) use primary::TurnPrimary;
pub(crate) use token::grant_token;
pub(crate) use wording::{grant_family, session_answers_line};
