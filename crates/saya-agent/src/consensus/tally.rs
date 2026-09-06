use crate::consensus::fingerprint::fingerprint;
use saya_types::QueryResult;

/// One candidate answer: the SQL that produced it and the result it returned.
///
/// `sql` is carried through untouched so the caller can recover the winning
/// statement; the vote never inspects it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub sql: String,
    /// `None` when this candidate failed to execute; it takes no part in the vote.
    pub result: Option<QueryResult>,
}

/// Outcome of a consensus vote over candidate answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consensus {
    /// Index into the input of the winning candidate, or `None` when nothing
    /// could win (no candidate executed, or the leaders tied).
    pub winner: Option<usize>,
    /// Votes for the leading group.
    pub votes: usize,
    /// Leading votes minus runner-up votes. `0` when the lead is tied.
    pub margin: usize,
    /// True when two or more groups tie for the lead.
    pub tied: bool,
    /// How many candidates failed to execute.
    pub failed: usize,
}

/// Vote over `candidates` by grouping their results by fingerprint.
///
/// `ordered` is passed through to [`fingerprint`]. The largest group wins; a
/// tie for the lead is not a win — the caller must defer rather than guess.
pub fn tally(candidates: &[Candidate], ordered: bool) -> Consensus {
    let mut failed = 0usize;
    // Groups keyed by fingerprint, in first-appearance order; each holds the
    // input indices of its members in input order.
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();

    for (idx, candidate) in candidates.iter().enumerate() {
        let Some(result) = candidate.result.as_ref() else {
            failed += 1;
            continue;
        };
        let fp = fingerprint(result, ordered);
        match groups.iter_mut().find(|(key, _)| key == &fp) {
            Some(group) => group.1.push(idx),
            None => groups.push((fp, vec![idx])),
        }
    }

    if groups.is_empty() {
        return Consensus {
            winner: None,
            votes: 0,
            margin: 0,
            tied: false,
            failed,
        };
    }

    let mut sizes: Vec<usize> = groups.iter().map(|(_, members)| members.len()).collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let leader = sizes[0];
    let runner_up = sizes.get(1).copied().unwrap_or(0);

    if leader == runner_up {
        // Two or more groups tie for the lead — do not pick arbitrarily.
        return Consensus {
            winner: None,
            votes: leader,
            margin: 0,
            tied: true,
            failed,
        };
    }

    let winner = groups
        .iter()
        .find(|(_, members)| members.len() == leader)
        .and_then(|(_, members)| members.first().copied())
        .expect("a unique leader exists, so a winning group is present");

    Consensus {
        winner: Some(winner),
        votes: leader,
        margin: leader - runner_up,
        tied: false,
        failed,
    }
}
