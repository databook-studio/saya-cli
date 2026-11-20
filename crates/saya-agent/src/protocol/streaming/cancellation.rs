use crate::BudgetDimension;
use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError, Weak,
    atomic::{AtomicU8, Ordering},
};
use tokio::sync::Notify;

/// A stop signal. A plain token is a leaf; a token made with
/// [`CancellationToken::child`] also stops when the token it was made from
/// stops.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<CancellationState>);

#[derive(Default)]
struct CancellationState {
    reason: AtomicU8,
    notify: Notify,
    /// The tokens this one stops with it, pruned whenever they are read: a
    /// parent whose children were all dropped holds nothing for them.
    children: Mutex<Vec<Weak<CancellationState>>>,
    /// The run an armed turn reports its non-budget stops to. Unset on a plain
    /// token and on a plain child.
    run: Mutex<Option<Arc<CancellationState>>>,
}

/// The first cause that cancelled an answering request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CancellationReason {
    User,
    Budget(BudgetDimension),
    OwnerFailure,
}

impl CancellationReason {
    const fn code(self) -> u8 {
        match self {
            Self::User => 1,
            Self::Budget(BudgetDimension::LogicalRequests) => 2,
            Self::Budget(BudgetDimension::RequestedToolCalls) => 3,
            Self::Budget(BudgetDimension::ElapsedTime) => 4,
            Self::Budget(BudgetDimension::ReportedTokens) => 5,
            Self::Budget(BudgetDimension::DownloadBytes) => 6,
            Self::OwnerFailure => 7,
            Self::Budget(BudgetDimension::RequestContextBytes) => 8,
            Self::Budget(BudgetDimension::RequestContextWindow) => 9,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::User),
            2 => Some(Self::Budget(BudgetDimension::LogicalRequests)),
            3 => Some(Self::Budget(BudgetDimension::RequestedToolCalls)),
            4 => Some(Self::Budget(BudgetDimension::ElapsedTime)),
            5 => Some(Self::Budget(BudgetDimension::ReportedTokens)),
            6 => Some(Self::Budget(BudgetDimension::DownloadBytes)),
            7 => Some(Self::OwnerFailure),
            8 => Some(Self::Budget(BudgetDimension::RequestContextBytes)),
            9 => Some(Self::Budget(BudgetDimension::RequestContextWindow)),
            _ => None,
        }
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that stops when this one does — including when this one was
    /// already stopped — and that never stops this one back. The child is
    /// tracked until it is dropped, and no longer.
    pub fn child(&self) -> Self {
        let child = Self::default();
        child.follows(self);
        child
    }

    /// The link [`CancellationToken::child`] creates, for a token that already
    /// exists: a stop of `parent` stops this token, and a stop of this token
    /// never reaches `parent`.
    pub(crate) fn follows(&self, parent: &CancellationToken) {
        self.attached_to(parent, false);
    }

    /// A turn's stop signal, armed on this run token: a stop of the run stops
    /// the turn, a budget stop stays in the turn — that is the whole point of
    /// arming one — and every other stop of the turn is the run's too, so the
    /// token the caller holds reads a user stop or an owner failure exactly as
    /// it did when a turn borrowed that token directly.
    pub fn turn_of(&self) -> Self {
        let turn = Self::default();
        turn.armed_on(self);
        turn
    }

    /// Arms this token as a turn of `run`, the link [`CancellationToken::turn_of`]
    /// gives a fresh one. A budget stop of this token is the turn's own and
    /// never reaches `run`.
    fn armed_on(&self, run: &CancellationToken) {
        self.attached_to(run, true);
    }

    /// How many live children this token tracks. A bound an owner can read: a
    /// token whose children were dropped tracks none of them.
    pub fn live_children(&self) -> usize {
        let mut children = self.0.children();
        children.retain(|child| child.strong_count() > 0);
        children.len()
    }

    /// Links this token into `parent`: a stop of `parent` stops this token with
    /// the parent's reason, and this token's own stops reach `parent` only when
    /// it is armed (`report`).
    fn attached_to(&self, parent: &CancellationToken, report: bool) {
        if Arc::ptr_eq(&self.0, &parent.0) {
            return;
        }
        parent.0.track(Arc::downgrade(&self.0));
        if report {
            self.0.report_to(&parent.0);
        }
        if let Some(reason) = parent.reason() {
            self.cancel_with_reason(reason);
        }
    }

    pub fn cancel(&self) {
        self.cancel_with_reason(CancellationReason::User);
    }
    /// Latches the first typed stop reason; a later cancellation cannot replace
    /// it, and only the first one propagates — to the children, and to the run
    /// of an armed turn.
    pub fn cancel_with_reason(&self, reason: CancellationReason) -> bool {
        let first = self
            .0
            .reason
            .compare_exchange(0, reason.code(), Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        self.0.notify.notify_waiters();
        if first {
            self.0.stop_children(reason);
            self.0.stop_run(reason);
        }
        first
    }
    pub fn reason(&self) -> Option<CancellationReason> {
        CancellationReason::from_code(self.0.reason.load(Ordering::Acquire))
    }
    pub fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }
    pub async fn cancelled(&self) {
        let notified = self.0.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

impl CancellationState {
    fn children(&self) -> MutexGuard<'_, Vec<Weak<CancellationState>>> {
        self.children.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers `child` as one to stop with this token, forgetting the ones
    /// that were already dropped so the list never outlives them.
    fn track(&self, child: Weak<CancellationState>) {
        let mut children = self.children();
        children.retain(|tracked| tracked.strong_count() > 0);
        children.push(child);
    }

    /// Stops every live child with `reason` and forgets the rest. The list is
    /// taken before the children are stopped, so a child's own stop never runs
    /// under this lock; a child registered meanwhile sees the reason already
    /// latched and stops itself.
    fn stop_children(&self, reason: CancellationReason) {
        let tracked = {
            let mut children = self.children();
            std::mem::take(&mut *children)
        };
        let mut live = Vec::with_capacity(tracked.len());
        for child in tracked {
            if let Some(child) = child.upgrade() {
                CancellationToken(Arc::clone(&child)).cancel_with_reason(reason);
                live.push(Arc::downgrade(&child));
            }
        }
        self.children().extend(live);
    }

    /// Remembers `run` as the run this token is armed on.
    fn report_to(&self, run: &Arc<CancellationState>) {
        *self.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(run));
    }

    /// Reports a stop to the run of an armed turn. A budget stop is the turn's
    /// own and stays here; every other stop ends the run, which is what a call
    /// on the run's token always did.
    fn stop_run(&self, reason: CancellationReason) {
        if matches!(reason, CancellationReason::Budget(_)) {
            return;
        }
        let run = self
            .run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(run) = run {
            CancellationToken(run).cancel_with_reason(reason);
        }
    }
}
