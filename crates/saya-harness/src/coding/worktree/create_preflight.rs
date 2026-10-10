//! Create-admission preflights: the cross-job checkout-id ownership rule
//! (an id is owned by one job for ever), the durable job binding, and the
//! agent registration, checked against durable state before the ledger is
//! touched.

use super::create::CreateCheckoutInput;
use super::create_error::CreateCheckoutError;
use super::create_events::inventory_identity;
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::{AgentId, JobId, JobState, ManagedCheckoutId};

/// Page size for the agent-registration preflight; the store's coding page
/// cap is 100.
const CODING_CHILDREN_PAGE: usize = 100;

/// The job's durable binding must name the repository identity this create
/// re-derives from the live inventory and the snapshot the plan will write;
/// any difference is the predictable store refusal (`Conflict`), refused
/// before the append. The store keeps the same checks for races.
pub(super) async fn check_job_binding(
    input: &CreateCheckoutInput<'_>,
    cancellation: &CancellationToken,
) -> Result<(), CreateCheckoutError> {
    let (repository_id, _common_dir_sha256) = inventory_identity(input, cancellation).await?;
    let job = match input.store.get_coding_job(input.job_id).await? {
        Some(job) if job.state == JobState::Running => job,
        _ => return Err(CreateCheckoutError::JobNotRunning),
    };
    if job.repository_id != repository_id || job.snapshot_id != *input.plan.snapshot_id() {
        return Err(CreateCheckoutError::JobBindingMismatch);
    }
    Ok(())
}

/// A supplied agent must be registered for the job, paged with the store's
/// bounded keyset cursor until it is found or the list ends. The store
/// keeps the same existence check for races.
pub(super) async fn check_agent_registration(
    input: &CreateCheckoutInput<'_>,
) -> Result<(), CreateCheckoutError> {
    let Some(agent_id) = input.agent_id.as_ref() else {
        return Ok(());
    };
    let mut cursor: Option<AgentId> = None;
    loop {
        let page = input
            .store
            .list_coding_children(input.job_id, cursor.as_ref(), CODING_CHILDREN_PAGE)
            .await?;
        if page.items.iter().any(|child| child.agent_id == *agent_id) {
            return Ok(());
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Err(CreateCheckoutError::AgentNotRegistered),
        }
    }
}

/// The checkout id is owned by one job for ever: a create whose id is
/// already durably owned by a different job is refused before anything is
/// read or sequenced. The store keeps the same ownership check for races.
pub(super) async fn check_checkout_identity(
    store: &SqliteStateStore,
    job_id: &JobId,
    checkout_id: &ManagedCheckoutId,
) -> Result<(), CreateCheckoutError> {
    match store.managed_checkout_owner(checkout_id).await? {
        Some(owner) if owner != *job_id => Err(CreateCheckoutError::CheckoutIdInUse),
        _ => Ok(()),
    }
}
