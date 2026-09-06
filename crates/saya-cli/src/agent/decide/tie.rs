//! Tie resolution for [`super::decide`].
//!
//! When the vote ties, this consults fan-out evidence to try to break it. A
//! probe that cannot be built, a statement that fails to execute, or a result
//! that is not a single numeric cell is NO SIGNAL — never read as fan-out.

use saya_agent::{CancellationToken, Candidate, fingerprint};
use saya_connectors::FanoutProbe;
use saya_types::QueryResult;
use serde_json::Value;

use super::CandidateExecutor;

/// Representative (lowest input index) of each tied leading group, found by
/// re-grouping with the same `fingerprint` and `ordered` flag `tally` used.
pub(super) fn tied_leaders(candidates: &[Candidate], ordered: bool, votes: usize) -> Vec<usize> {
    let mut groups: Vec<(String, usize)> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();
    for (idx, c) in candidates.iter().enumerate() {
        let Some(result) = c.result.as_ref() else {
            continue;
        };
        let fp = fingerprint(result, ordered);
        match groups.iter().position(|(key, _)| key == &fp) {
            Some(i) => sizes[i] += 1,
            None => {
                groups.push((fp, idx));
                sizes.push(1);
            }
        }
    }
    sizes
        .iter()
        .enumerate()
        .filter(|(_, size)| **size == votes)
        .map(|(i, _)| groups[i].1)
        .collect()
}

pub(super) enum ProbeOutcome {
    Cancelled,
    NoSignal,
    Flagged(bool),
}

/// Run one fan-out probe. Either statement failing, or a result that is not a
/// single numeric cell, is NO SIGNAL — never read as fan-out. The cancellation
/// token is checked before each executor call.
pub(super) async fn probe_one(
    executor: &dyn CandidateExecutor,
    probe: &FanoutProbe,
    cancellation: &CancellationToken,
) -> ProbeOutcome {
    if cancellation.is_cancelled() {
        return ProbeOutcome::Cancelled;
    }
    let joined = executor.run(&probe.joined_rows).await;
    if cancellation.is_cancelled() {
        return ProbeOutcome::Cancelled;
    }
    let base = executor.run(&probe.base_rows).await;
    let (Some(joined), Some(base)) = (joined, base) else {
        return ProbeOutcome::NoSignal;
    };
    let (Some(j), Some(b)) = (single_count(&joined), single_count(&base)) else {
        return ProbeOutcome::NoSignal;
    };
    ProbeOutcome::Flagged(j > b)
}

/// Extract a single integral count from a `SELECT COUNT(*) AS n` result.
/// Anything that is not exactly one row, one column, one numeric cell is no
/// signal. Rows are normally JSON arrays (`[n]`); a bare scalar is also accepted.
fn single_count(result: &QueryResult) -> Option<i64> {
    if result.rows.len() != 1 || result.columns.len() != 1 {
        return None;
    }
    let cell = match &result.rows[0] {
        Value::Array(cells) if cells.len() == 1 => &cells[0],
        Value::Array(_) => return None,
        other => other,
    };
    match cell {
        Value::Number(num) => num
            .as_i64()
            .or_else(|| num.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        _ => None,
    }
}

/// Break a tie when exactly one leader is cleared while every other is flagged.
/// Any other spread of evidence — none cleared, several cleared, or any `None`
/// — is ambiguous and leaves the tie unbroken.
pub(super) fn break_tie(leaders: &[usize], fanout: &[Option<bool>]) -> Option<usize> {
    let flag = |i: usize| fanout.get(i).copied().flatten();
    let cleared: Vec<usize> = leaders
        .iter()
        .copied()
        .filter(|&i| flag(i) == Some(false))
        .collect();
    if cleared.len() != 1 {
        return None;
    }
    let winner = cleared[0];
    let others_all_flagged = leaders
        .iter()
        .copied()
        .filter(|&i| i != winner)
        .all(|i| flag(i) == Some(true));
    others_all_flagged.then_some(winner)
}
