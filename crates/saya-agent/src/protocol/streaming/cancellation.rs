use crate::BudgetDimension;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<CancellationState>);
#[derive(Default)]
struct CancellationState {
    reason: AtomicU8,
    notify: Notify,
}

/// The first cause that cancelled an answering request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub fn cancel(&self) {
        self.cancel_with_reason(CancellationReason::User);
    }
    /// Latches the first typed stop reason; a later cancellation cannot replace it.
    pub fn cancel_with_reason(&self, reason: CancellationReason) -> bool {
        let first = self
            .0
            .reason
            .compare_exchange(0, reason.code(), Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        self.0.notify.notify_waiters();
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
