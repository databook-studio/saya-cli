use std::future::Future;

use saya_agent::CancellationToken;
use tokio::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplayStop {
    Cancelled,
    TimedOut,
}

pub(super) enum ReplayCompletion<T> {
    Completed(T),
    Cancelled(Option<T>),
    TimedOut(Option<T>),
}

pub(super) const fn stop_message(stop: ReplayStop) -> &'static str {
    match stop {
        ReplayStop::Cancelled => "the call was cancelled",
        ReplayStop::TimedOut => {
            "the database operation exceeded its 30-second cancellation-request deadline"
        }
    }
}

pub(super) async fn acquire_replay_slot<F, C>(
    slot: &Mutex<()>,
    source_cancelled: F,
    source_is_cancelled: C,
    deadline: tokio::time::Instant,
) -> Result<MutexGuard<'_, ()>, ReplayStop>
where
    F: Future<Output = ()>,
    C: Fn() -> bool,
{
    if source_is_cancelled() {
        return Err(ReplayStop::Cancelled);
    }
    if tokio::time::Instant::now() >= deadline {
        return Err(ReplayStop::TimedOut);
    }
    tokio::select! {
        biased;
        _ = source_cancelled => Err(ReplayStop::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(ReplayStop::TimedOut),
        guard = slot.lock() => Ok(guard),
    }
}

pub(super) async fn retain_replay<F, C, S, T>(
    operation: F,
    cancellation: &CancellationToken,
    source_cancelled: C,
    source_is_cancelled: S,
    deadline: tokio::time::Instant,
) -> ReplayCompletion<T>
where
    F: Future<Output = T>,
    C: Future<Output = ()>,
    S: Fn() -> bool,
{
    if source_is_cancelled() {
        cancellation.cancel();
        return ReplayCompletion::Cancelled(None);
    }
    if tokio::time::Instant::now() >= deadline {
        cancellation.cancel();
        return ReplayCompletion::TimedOut(None);
    }

    tokio::pin!(operation);
    tokio::select! {
        biased;
        _ = source_cancelled => {
            cancellation.cancel();
            ReplayCompletion::Cancelled(Some(operation.await))
        }
        _ = tokio::time::sleep_until(deadline) => {
            cancellation.cancel();
            ReplayCompletion::TimedOut(Some(operation.await))
        }
        result = &mut operation => ReplayCompletion::Completed(result),
    }
}
