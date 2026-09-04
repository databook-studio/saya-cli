//! Decide which nominated answer to believe.
//!
//! Joins [`saya_agent::consensus`] (vote by result-set fingerprint) with
//! [`saya_connectors::verify`] (fan-out probe): run each nominated statement,
//! tally fingerprints, and — only on a tie — consult fan-out evidence to break
//! it. This module decides; it generates no candidates and touches no agent
//! loop, config, or CLI. Nothing here runs unless [`decide`] is called.

use async_trait::async_trait;
use saya_agent::{CancellationToken, Candidate, tally};
use saya_connectors::{fanout_probe, has_top_level_order_by};
use saya_types::{QueryResult, SqlDialect};

mod tie;

/// Runs one read-only statement and returns its result, or `None` when it
/// failed for any reason. Implementations go through the normal bounded
/// read-only path; this trait exists so the decision logic can be tested
/// against a fake.
#[async_trait]
pub(crate) trait CandidateExecutor: Sync {
    async fn run(&self, sql: &str) -> Option<QueryResult>;
}

/// Outcome of deciding among nominated answers.
pub(crate) struct Decision {
    /// Index of the candidate to answer with, or `None` when the evidence does
    /// not support choosing one.
    pub winner: Option<usize>,
    pub votes: usize,
    pub margin: usize,
    pub tied: bool,
    /// True when a tie was resolved by fan-out evidence rather than by votes.
    pub probe_broke_tie: bool,
    /// Per-candidate: `Some(true)` flagged as fanned out, `Some(false)` cleared,
    /// `None` when no sound probe could be built or the probe did not run.
    pub fanout: Vec<Option<bool>>,
}

/// Decide which nominated SQL to believe. `nominated[i]` is the SQL run `i`
/// designated, or `None` when that run designated nothing.
pub(crate) async fn decide(
    nominated: &[Option<String>],
    executor: &dyn CandidateExecutor,
    dialect: SqlDialect,
    cancellation: &CancellationToken,
) -> Decision {
    let n = nominated.len();
    let mut candidates: Vec<Candidate> = Vec::with_capacity(n);
    for nominated_sql in nominated {
        let Some(sql) = nominated_sql.as_deref() else {
            // A None nomination: no SQL, no result — it takes no part in the vote.
            candidates.push(Candidate {
                sql: String::new(),
                result: None,
            });
            continue;
        };
        if cancellation.is_cancelled() {
            return cancelled(n);
        }
        // A statement that failed to execute also becomes result: None.
        let result = executor.run(sql).await;
        candidates.push(Candidate {
            sql: sql.to_string(),
            result,
        });
    }

    // `ordered`: compare as ordered when ANY executed candidate has a top-level
    // ORDER BY. The asymmetry is deliberate and safe by design: comparing as
    // ordered when order does not matter only splits a group and yields a tie,
    // and a tie defers; comparing as unordered when order DOES matter merges
    // genuinely different answers and can hand back a wrongly ordered one.
    let ordered = candidates
        .iter()
        .any(|c| c.result.is_some() && has_top_level_order_by(&c.sql, dialect));

    let consensus = tally(&candidates, ordered);
    if !consensus.tied {
        // No tie: the vote decides. No probe runs — nothing depends on it and
        // it costs database queries.
        return Decision {
            winner: consensus.winner,
            votes: consensus.votes,
            margin: consensus.margin,
            tied: false,
            probe_broke_tie: false,
            fanout: vec![None; n],
        };
    }

    // Tie: consult fan-out evidence for one representative (the lowest input
    // index) of each tied leading group.
    let leaders = tie::tied_leaders(&candidates, ordered, consensus.votes);
    let mut fanout = vec![None; n];
    for &rep in &leaders {
        let candidate = &candidates[rep];
        if candidate.result.is_none() {
            continue; // no executed SQL to probe; fanout[rep] stays None
        }
        let Some(probe) = fanout_probe(&candidate.sql, dialect) else {
            continue; // no sound probe; fanout[rep] stays None
        };
        match tie::probe_one(executor, &probe, cancellation).await {
            tie::ProbeOutcome::Cancelled => return cancelled(n),
            tie::ProbeOutcome::NoSignal => {} // fanout[rep] stays None
            tie::ProbeOutcome::Flagged(f) => fanout[rep] = Some(f),
        }
    }
    let winner = tie::break_tie(&leaders, &fanout);
    Decision {
        winner,
        votes: consensus.votes,
        margin: consensus.margin,
        tied: true,
        probe_broke_tie: winner.is_some(),
        fanout,
    }
}

/// A cancelled decision is never a considered one.
fn cancelled(n: usize) -> Decision {
    Decision {
        winner: None,
        votes: 0,
        margin: 0,
        tied: false,
        probe_broke_tie: false,
        fanout: vec![None; n],
    }
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
