use super::{CodingError, NativeCodingJob};
use saya_harness::coding::coordinator::replay_completions;
use serde_json::{Value, json};

const MAX_REPLAYED_FINDINGS: usize = 32;

impl NativeCodingJob {
    /// The results no parent has accepted yet, replayed by their stable
    /// (job, sequence) identity so a resumed root can read each one with
    /// `coding_result` and accept it explicitly. Replay carries identity and
    /// outcome only — never answer content, never authority — and reading it
    /// accepts nothing. Returns the listed entries and how many open results
    /// did not fit the bound.
    pub(super) async fn unaccepted_findings(
        &self,
        open_total: usize,
    ) -> Result<(Vec<Value>, usize), CodingError> {
        let page = replay_completions(
            &self.application.store,
            self.job_id(),
            None,
            MAX_REPLAYED_FINDINGS,
        )
        .await
        .map_err(CodingError::owner)?;
        let listed: Vec<Value> = page
            .items
            .iter()
            .map(|completion| {
                json!({
                    "sequence": completion.sequence,
                    "agent_id": completion.result.agent_id(),
                    "outcome": completion.result.outcome()
                })
            })
            .collect();
        let omitted = open_total.saturating_sub(listed.len());
        Ok((listed, omitted))
    }
}
