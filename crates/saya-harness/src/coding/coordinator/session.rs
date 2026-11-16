//! A long-lived schedule session: one lease held for its whole life, children
//! admitted while it runs, each with its own completion signal.
//!
//! The one-shot `run_schedule` is a session opened on one batch and closed
//! immediately. A job holds one session open across a whole run so a later
//! `helpers` call joins the children already running instead of being refused.
//!
//! A child's own failure is its own: it is recorded against that child and its
//! siblings keep running. Only an owner failure, the caller's cancellation, the
//! deadline, or a lost lease stop the whole session. The running half is
//! `session_loop.rs`.
use super::super::budget::JobBudgetLedger;
use super::{
    admit::{self, NodePlan, NodeState, SessionIndex},
    api::ChildRunError,
    events::ChildEvent,
    schedule_types::{
        ChildOutcome, MAX_SCHEDULE_EVENT_CAPACITY, MAX_SCHEDULE_TASKS, ScheduleError,
        ScheduleLimits, ScheduleOutcome, ScheduleReport, ScheduleStop, ScheduledChild,
    },
};
use crate::coding::budget::ScheduleLease;
use saya_agent::{AgentOutput, CancellationToken};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::{Id, JoinSet},
    time::Instant,
};

/// The submit side of a session, held by the job while the session runs. Cheap
/// to clone; the session is closed by dropping every clone.
#[derive(Clone)]
pub struct ScheduleSession {
    submit: mpsc::Sender<SessionChild>,
    index: Arc<Mutex<SessionIndex>>,
}

impl ScheduleSession {
    /// Opens a session on its first batch, returning its submit side, the
    /// running half, and one completion signal per child of the first batch in
    /// the order they were given. The batch's identity is validated against the
    /// ledger, the limits against job capacity, and the one schedule lease is
    /// taken here and held until the session drains. Every later child is
    /// validated against this first batch's identity.
    #[allow(clippy::type_complexity)]
    pub async fn open(
        first: Vec<ScheduledChild>,
        limits: ScheduleLimits,
        ledger: &JobBudgetLedger,
        caller: CancellationToken,
        progress: mpsc::Sender<ChildEvent>,
    ) -> Result<
        (
            ScheduleSession,
            SessionRun,
            Vec<oneshot::Receiver<ChildOutcome>>,
        ),
        ScheduleError,
    > {
        let plans = admit::validate(&first, &ledger.job_id().await)?;
        let uncapped = ledger.is_uncapped().await;
        if first
            .iter()
            .any(|node| node.spec.aggregate_uncapped() != uncapped)
        {
            return Err(ScheduleError::PolicyMismatch);
        }
        let capacity = ledger.limits().await;
        let children =
            u64::try_from(limits.max_active_children).map_err(|_| ScheduleError::InvalidLimits)?;
        if children == 0
            || children > capacity.child_slots
            || children > capacity.model_slots.saturating_sub(1)
            || limits.max_active_per_provider == 0
            || limits.max_active_per_provider > limits.max_active_children
            || progress.max_capacity() > MAX_SCHEDULE_EVENT_CAPACITY
        {
            return Err(ScheduleError::InvalidLimits);
        }
        let deadline = match ledger.remaining().await {
            Some(remaining) => Some(
                Instant::now()
                    .checked_add(remaining)
                    .ok_or(ScheduleError::InvalidLimits)?,
            ),
            None => None,
        };
        let owner = ledger.cancellation_token();
        if owner.is_cancelled() {
            return Err(ScheduleError::OwnerUnavailable);
        }
        let lease = ledger.try_schedule().ok_or(ScheduleError::Busy)?;
        if owner.is_cancelled() {
            return Err(ScheduleError::OwnerUnavailable);
        }
        if ledger.snapshot().await.child_slots_active != 0 {
            return Err(ScheduleError::ActiveChildren);
        }
        ledger
            .bind_runtime(
                first[0].spec.authority().revocation_epoch(),
                first[0].spec.authority().policy_digest(),
            )
            .await
            .map_err(|_| ScheduleError::FreshAuthorityRequired)?;
        let identity = admit::SessionIdentity::of(&first[0]);
        let index = Arc::new(Mutex::new(SessionIndex::of(&plans, identity)));
        let (submit, receiver) = mpsc::channel(MAX_SCHEDULE_TASKS);
        let (waiters, completions) = (0..plans.len())
            .map(|_| {
                let (waiter, completion) = oneshot::channel();
                (Some(waiter), completion)
            })
            .unzip();
        let state = Session {
            nodes: first.into_iter().map(Some).collect(),
            states: vec![NodeState::Pending; plans.len()],
            waiters,
            plans,
            limits,
            ledger: ledger.clone(),
            owner,
            caller,
            deadline,
            progress,
            cancellation: CancellationToken::new(),
            lease: Some(lease),
            receiver,
            children: JoinSet::new(),
            active: HashMap::new(),
            outcomes: Vec::new(),
            stop: None,
            settled: true,
            closed: false,
        };
        Ok((Self { submit, index }, SessionRun { state }, completions))
    }

    /// Submits one more child while the session runs. The returned receiver
    /// carries that child's own completion; it resolves to `Err` only if the
    /// session ended without starting the child.
    pub async fn submit(
        &self,
        child: ScheduledChild,
    ) -> Result<oneshot::Receiver<ChildOutcome>, ScheduleError> {
        let plan = self
            .index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .admit(&child)?;
        let (waiter, completion) = oneshot::channel();
        self.submit
            .send(SessionChild {
                node: child,
                plan,
                waiter,
            })
            .await
            .map_err(|_| ScheduleError::SessionClosed)?;
        Ok(completion)
    }
}

/// The running half of a session. It owns the lease, so dropping this future
/// aborts its children and fails the ledger closed, exactly as dropping the
/// one-shot schedule does.
pub struct SessionRun {
    state: Session,
}

impl SessionRun {
    /// Drives the session until it is closed and drained. Running children
    /// always finish before the report is returned.
    pub async fn run(self) -> Result<ScheduleReport, ScheduleError> {
        self.state.run().await
    }
}

/// One child on its way into a running session: its node, the plan resolved
/// against the session's identity, and the waiter its own completion fires.
pub(super) struct SessionChild {
    pub(super) node: ScheduledChild,
    pub(super) plan: NodePlan,
    pub(super) waiter: oneshot::Sender<ChildOutcome>,
}

/// One running session: the children it holds, the lease it keeps for its whole
/// life, and the report it is building. Driven by `session_loop.rs`.
pub(super) struct Session {
    pub(super) nodes: Vec<Option<ScheduledChild>>,
    pub(super) plans: Vec<NodePlan>,
    pub(super) states: Vec<NodeState>,
    pub(super) waiters: Vec<Option<oneshot::Sender<ChildOutcome>>>,
    pub(super) limits: ScheduleLimits,
    pub(super) ledger: JobBudgetLedger,
    pub(super) owner: CancellationToken,
    pub(super) caller: CancellationToken,
    pub(super) deadline: Option<Instant>,
    pub(super) progress: mpsc::Sender<ChildEvent>,
    pub(super) cancellation: CancellationToken,
    pub(super) lease: Option<ScheduleLease>,
    pub(super) receiver: mpsc::Receiver<SessionChild>,
    pub(super) children: JoinSet<Result<AgentOutput, ChildRunError>>,
    pub(super) active: HashMap<Id, usize>,
    pub(super) outcomes: Vec<ScheduleOutcome>,
    pub(super) stop: Option<ScheduleStop>,
    pub(super) settled: bool,
    pub(super) closed: bool,
}
