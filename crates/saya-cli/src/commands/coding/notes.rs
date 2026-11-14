use super::{CodingApplication, CodingError, NativeCodingJob};
use saya_types::JobId;
use serde_json::{Value, json};
use std::num::NonZeroU64;

impl CodingApplication {
    /// Child reports are task-scoped untrusted notes, never implicit preferences.
    pub async fn notes(&self, job: &JobId) -> Result<Value, CodingError> {
        self.status(job).await?;
        let page = self
            .store
            .list_coding_completion_history(job, None, 32)
            .await
            .map_err(CodingError::owner)?;
        Ok(
            json!({"authority":"none","findings":page.items.iter().map(|item|{
                let mut finding = json!({"sequence":item.sequence,"report":item.result.answer(),"outcome":item.result.outcome(),"accepted":item.acknowledged});
                let added = super::assignment_added::assignment_added_lines(item.result.answer());
                if !added.is_empty() {
                    finding["assignment_added"] = json!(added);
                }
                finding
            }).collect::<Vec<_>>(),"next_cursor":page.next_cursor}),
        )
    }
    /// Only explicit user-selected text can reach the existing preference owner.
    /// No model tool exposes this operation, and a completion receipt grants none.
    pub async fn promote_finding(
        &self,
        job: &JobId,
        sequence: NonZeroU64,
        text: &str,
        confirmed: bool,
    ) -> Result<Value, CodingError> {
        if !confirmed {
            return Err(CodingError::ApprovalRequired);
        }
        self.status(job).await?;
        let completion = self
            .store
            .get_coding_completion(job, sequence)
            .await
            .map_err(CodingError::owner)?
            .ok_or(CodingError::NotOwned)?;
        if !completion.acknowledged {
            return Err(CodingError::ApprovalRequired);
        }
        let worktree = crate::commands::preferences::worktree_id_for(self.root_path())
            .map_err(|_| CodingError::NotOwned)?;
        let result =
            crate::commands::preferences::operations::add_preference(&self.store, &worktree, text)
                .await;
        let (code, message) = crate::commands::preferences::render::render(&result);
        if code != 0 {
            return Err(CodingError::InvalidRequest);
        }
        Ok(json!({"promoted":true,"message":message,"authority":"none"}))
    }
}
impl NativeCodingJob {
    pub async fn accept_finding(&self, sequence: NonZeroU64) -> Result<bool, CodingError> {
        saya_harness::coding::coordinator::acknowledge_completion(
            &self.application.store,
            self.job_id(),
            sequence,
        )
        .await
        .map_err(CodingError::owner)
    }
}
