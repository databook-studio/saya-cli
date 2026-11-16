use saya_store::{CodingCompletionPage, SqliteStateStore, StoreError};
use saya_types::JobId;
use std::num::NonZeroU64;

/// Replays stable completion identities without repeating any child effects.
/// A parent accepts by (job, sequence), then ACKs. Reading is not acceptance.
pub async fn replay_completions(
    store: &SqliteStateStore,
    job: &JobId,
    after: Option<NonZeroU64>,
    limit: usize,
) -> Result<CodingCompletionPage, StoreError> {
    store.list_coding_completions(job, after, limit).await
}
pub async fn acknowledge_completion(
    store: &SqliteStateStore,
    job: &JobId,
    sequence: NonZeroU64,
) -> Result<bool, StoreError> {
    store.ack_coding_completion(job, sequence).await
}
