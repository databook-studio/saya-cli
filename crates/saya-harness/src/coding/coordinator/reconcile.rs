//! Coordination-side reconciliation of a job's unresolved budget effects.
//! This operation never refunds a charge, never kills a process, never
//! re-runs an effect, and never grants authority: a resumed job still
//! needs fresh runtime approval through `resume_job`.

use std::num::NonZeroU64;

use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{BudgetCharge, CodingEventKind, CodingEventV1, JobId, JobState, Sha256Digest};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::recovery::inspect_recovery;

/// One unresolved budget effect of a job: an open reservation (an effect
/// whose process died before settling) or one settled `Uncertain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnresolvedEffect {
    pub reservation_sequence: NonZeroU64,
    pub settled_uncertain: bool,
    pub reserved: Vec<BudgetCharge>,
}

/// The exact plan for reconciling one job's unresolved effects. The
/// digest is the SHA-256 of the compact JSON of the job id, the stream
/// length, and every effect's identity and charges, so it changes
/// whenever anything unresolved or the event stream length changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationPlan {
    pub job: JobId,
    pub last_sequence: u64,
    pub effects: Vec<UnresolvedEffect>,
    pub digest: Sha256Digest,
}

/// The effects reconciled by one [`reconcile_effects`] call, in the
/// order their reconciliation events were appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub reconciled: Vec<NonZeroU64>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReconcileError {
    #[error("job state is neither Running nor Paused, so its effects cannot be reconciled")]
    NotReconcilable,
    #[error("job still has active children")]
    ChildrenActive,
    #[error("more than 256 unresolved effects")]
    TooManyEffects,
    #[error("expected plan digest does not match the current plan")]
    StalePlan,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The private digest input: exactly the plan's load-bearing parts,
/// serialised compactly. Its keys fix the digest's meaning forever.
#[derive(Serialize)]
struct PlanDigestWire<'a> {
    job: &'a JobId,
    last_sequence: u64,
    effects: &'a [UnresolvedEffect],
}

const MAX_PLANNED_EFFECTS: usize = 256;
const UNRESOLVED_PAGE: usize = 100;

/// Lists a job's unresolved effects exactly as recovery counts them and
/// seals the list behind a digest. Nothing is written: the caller holds
/// the plan and decides with [`reconcile_effects`].
pub async fn plan_reconciliation(
    store: &SqliteStateStore,
    job: &JobId,
) -> Result<ReconciliationPlan, ReconcileError> {
    let status = inspect_recovery(store, job).await?;
    if !matches!(status.state, JobState::Running | JobState::Paused) {
        return Err(ReconcileError::NotReconcilable);
    }
    if status.active_children > 0 {
        return Err(ReconcileError::ChildrenActive);
    }
    let mut cursor = None;
    let mut effects = Vec::new();
    loop {
        let page = store
            .list_unresolved_coding_reservations(job, cursor, UNRESOLVED_PAGE)
            .await?;
        for record in &page.items {
            effects.push(UnresolvedEffect {
                reservation_sequence: record.reservation.reservation_sequence(),
                settled_uncertain: record.settlement.is_some(),
                reserved: record.reservation.charges().to_vec(),
            });
            if effects.len() > MAX_PLANNED_EFFECTS {
                return Err(ReconcileError::TooManyEffects);
            }
        }
        match page.next_cursor {
            None => break,
            Some(next) if Some(next) != cursor => cursor = Some(next),
            Some(_) => return Err(ReconcileError::Store(StoreError::Invalid)),
        }
    }
    let digest = plan_digest(job, status.last_sequence, &effects)?;
    Ok(ReconciliationPlan {
        job: job.clone(),
        last_sequence: status.last_sequence,
        effects,
        digest,
    })
}

/// Reconciles every effect of the plan sealed by `expected`: for each
/// one a job-level `BudgetReconciled` event is appended at the next
/// stream sequence and the reservation is marked reconciled, so recovery
/// stops counting it. Nothing here refunds a charge, releases a slot,
/// re-runs an effect, or kills a process; resume afterwards still needs
/// fresh approval. Use it only for a job that is not running anywhere:
/// the store cannot tell a crashed effect from a live in-flight one. If
/// the job did append events since the plan, this call loses the race
/// (`StalePlan`, or a store conflict) and writes nothing further; if this
/// call appends first, the live job's next append conflicts and its own
/// ledger fails closed. Effects reconciled before a failure stay
/// reconciled, and a fresh plan shows the remainder.
pub async fn reconcile_effects(
    store: &SqliteStateStore,
    job: &JobId,
    expected: &Sha256Digest,
) -> Result<ReconcileReport, ReconcileError> {
    let plan = plan_reconciliation(store, job).await?;
    if plan.digest != *expected {
        return Err(ReconcileError::StalePlan);
    }
    let mut reconciled = Vec::with_capacity(plan.effects.len());
    for (index, effect) in plan.effects.iter().enumerate() {
        let event = CodingEventV1::new(
            event_sequence(&plan, index)?,
            job.clone(),
            None,
            None,
            CodingEventKind::BudgetReconciled {
                reservation_sequence: effect.reservation_sequence,
                note_sha256: plan.digest.clone(),
            },
        )
        .map_err(|_| ReconcileError::Store(StoreError::Invalid))?;
        match store.reconcile_coding_budget(event).await? {
            CodingEventAppend::Appended(_) => {}
            CodingEventAppend::AlreadyPresent(_) => {
                return Err(ReconcileError::StalePlan);
            }
            _ => return Err(ReconcileError::Store(StoreError::Conflict)),
        }
        reconciled.push(effect.reservation_sequence);
    }
    Ok(ReconcileReport { reconciled })
}

/// The digest over the private wire shape: compact JSON, SHA-256,
/// lowercase hex. Serialisation of this shape cannot fail; the error arm
/// only satisfies the store-typed failure channel.
fn plan_digest(
    job: &JobId,
    last_sequence: u64,
    effects: &[UnresolvedEffect],
) -> Result<Sha256Digest, ReconcileError> {
    let wire = PlanDigestWire {
        job,
        last_sequence,
        effects,
    };
    let bytes =
        serde_json::to_vec(&wire).map_err(|_| ReconcileError::Store(StoreError::Invalid))?;
    let hex = format!("{:x}", Sha256::digest(bytes));
    Sha256Digest::parse(&hex).map_err(|_| ReconcileError::Store(StoreError::Invalid))
}

/// The event sequence for effect `index`: the stream length when the
/// plan was taken, plus one, plus the effect's position — exactly the
/// sequences a live job's ledger would race for and refuse.
fn event_sequence(plan: &ReconciliationPlan, index: usize) -> Result<NonZeroU64, ReconcileError> {
    plan.last_sequence
        .checked_add(1)
        .and_then(|next| next.checked_add(index as u64))
        .and_then(NonZeroU64::new)
        .ok_or(ReconcileError::Store(StoreError::LimitExceeded))
}
