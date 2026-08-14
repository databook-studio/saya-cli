//! The candidate review queue: the opposite view of recall. Recall answers
//! "what is true about this question"; the queue answers "what is waiting for
//! me". It lists `Candidate` claims only, ordered so a reviewer works a stable
//! list, with the schema state and evidence count a human needs to decide.
//!
//! No policy of its own: candidates become recallable only through the existing
//! `review` operation (2b-2c), never here. See .claude/specs/spec-3d-review-queue.md.

use crate::contracts::validity::schema_state_for;
use crate::contracts::view::ContractSchemaState;
use saya_store::{ContractStore, SqliteStateStore, StoredClaim};
use saya_types::{ClaimStatus, ProfileIdentity, SchemaTree};

use crate::contracts::review::ContractOpError;

/// The largest queue a single read returns. A reviewer works the top of the
/// list; beyond this the list stops being a queue and becomes an archive.
pub(crate) const QUEUE_LIMIT_CAP: usize = 200;

/// The default queue size when the adapter does not name one. Capped well
/// under [`QUEUE_LIMIT_CAP`] so an unbounded `saya contracts queue` still
/// returns a workable list, not the whole archive.
pub(crate) const QUEUE_DEFAULT_LIMIT: usize = 50;

/// One candidate waiting for review. Carries the claim, the per-claim schema
/// state (a candidate about a table that has since changed says so), and the
/// evidence count that orders the queue. The evidence *rows* stay in the store
/// — they carry session ids and turn ordinals the queue does not need.
#[derive(Debug)]
pub(crate) struct QueuedCandidate {
    pub claim: StoredClaim,
    pub schema_state: ContractSchemaState,
    pub evidence_count: usize,
}

/// The candidate review queue for `profiles`. Candidates only, ordered most
/// evidence first, then oldest first, then by claim id string — a queue whose
/// order shifts between runs is one a user cannot work through. `limit` is
/// clamped to [`QUEUE_LIMIT_CAP`] on the upper bound; `0` is an empty queue,
/// not clamped up, because "nothing waiting" is a legitimate answer.
pub(crate) async fn review_queue(
    store: &SqliteStateStore,
    profiles: &[ProfileIdentity],
    schemas: &[(ProfileIdentity, SchemaTree)],
    limit: usize,
) -> Result<Vec<QueuedCandidate>, ContractOpError> {
    let limit = limit.min(QUEUE_LIMIT_CAP);
    let mut queued = Vec::new();
    for profile in profiles {
        for object in store.list_objects(profile).await? {
            for claim in store
                .list_claims(&object.object, &[ClaimStatus::Candidate])
                .await?
            {
                let evidence_count = store.evidence_count(&claim.id).await?;
                let live = live_schema(schemas, claim.object.profile());
                let schema_state = schema_state_for(&claim, live);
                queued.push(QueuedCandidate {
                    claim,
                    schema_state,
                    evidence_count,
                });
            }
        }
    }
    queued.sort_by(|a, b| {
        b.evidence_count
            .cmp(&a.evidence_count)
            .then(a.claim.created_unix_ms.cmp(&b.claim.created_unix_ms))
            .then(a.claim.id.as_str().cmp(b.claim.id.as_str()))
    });
    queued.truncate(limit);
    Ok(queued)
}

fn live_schema<'s>(
    schemas: &'s [(ProfileIdentity, SchemaTree)],
    profile: &ProfileIdentity,
) -> Option<&'s SchemaTree> {
    schemas
        .iter()
        .find(|(p, _)| p == profile)
        .map(|(_, tree)| tree)
}
