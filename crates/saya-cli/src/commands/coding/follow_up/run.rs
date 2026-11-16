//! Standalone adapters keep the same cancellation/settlement/final-state owner.
use super::{ChildFollowUpRequest, CodingError, NativeCodingJob};
use saya_agent::CancellationToken;
use serde_json::{Value, json};
use std::sync::Arc;
impl NativeCodingJob {
    pub async fn run_follow_up(
        self: &Arc<Self>,
        request: ChildFollowUpRequest,
    ) -> Result<Value, CodingError> {
        let receipt = self.follow_up(request).await?;
        let stop = CancellationToken::new();
        let mut guard = JoinGuard {
            root: self.cancellation.clone(),
            stop: stop.clone(),
            armed: true,
        };
        let join = async {
            let result = self.join().await.and_then(|report| {
                let mut outcomes = report.outcomes.into_iter();
                let outcome = outcomes.next().ok_or(CodingError::NotOwned)?;
                if outcomes.next().is_some() {
                    return Err(CodingError::InvalidRequest);
                }
                outcome.result.map_err(CodingError::owner)
            });
            stop.cancel();
            result
        };
        let (result, ()) = tokio::join!(join, self.watch_cancel(stop.clone()));
        // The follow-up runs in W like every other helper, so the work the
        // operator asked for is already in the checkout the candidate is built
        // from: there is no merge step and nothing to report back.
        let output = self.finalize_run(result).await?;
        guard.armed = false;
        let answer = self
            .outcome()
            .and_then(|outcome| outcome["answer"].as_str().map(str::to_owned))
            .unwrap_or(output.answer);
        Ok(
            json!({"job_id":self.job_id(),"follow_up":receipt,"answer":saya_types::redact(&answer),"truncated":output.truncated,"failure":self.failure()}),
        )
    }
}
struct JoinGuard {
    root: CancellationToken,
    stop: CancellationToken,
    armed: bool,
}
impl Drop for JoinGuard {
    fn drop(&mut self) {
        self.stop.cancel();
        if self.armed {
            self.root
                .cancel_with_reason(saya_agent::CancellationReason::OwnerFailure);
        }
    }
}
