//! The run's download budget: a shared wallet of bytes all of a run's
//! downloads spend from, tripped fail-safe — paused, not overrun.
//!
//! A chunk is claimed from the wallet *before* it is written, so the byte
//! that would exceed the budget is never written: a tripped download stops
//! with exactly the bytes under the limit, its partial left resumable. The
//! claim is atomic so concurrent tool calls in one turn cannot race the
//! wallet into an overrun. Claims are never refunded on failure — a download
//! that fails after claiming counts its claim, which can only bias the
//! budget toward stopping sooner (the safe direction), never toward an
//! overrun.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Provisional default for a run's total download bytes: a full corpus
/// artifact set, bounded so one run cannot silently fill a disk. Provisional
/// until M5 measures real runs (U8) — the number exists so the default is
/// bounded, not because it is measured.
pub const DEFAULT_MAX_RUN_BYTES: u64 = 1024 * 1024 * 1024;

/// The per-run download wallet, shared across a run's downloads.
#[derive(Clone, Debug)]
pub struct DownloadBudget {
    max_bytes: u64,
    consumed: Arc<AtomicU64>,
}

impl DownloadBudget {
    /// A fresh wallet of `max_bytes` for one run.
    pub fn new(max_bytes: u64) -> Self {
        Self {
            max_bytes,
            consumed: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The limit the wallet was built with.
    pub fn limit(&self) -> u64 {
        self.max_bytes
    }

    /// Bytes claimed so far across every download sharing this wallet.
    pub fn consumed(&self) -> u64 {
        self.consumed.load(Ordering::SeqCst)
    }

    /// Claims `bytes` for writing. `false` when the claim would exceed the
    /// limit — the caller must stop before writing (paused, not overrun).
    /// Cloning shares the same wallet; claims never exceed the limit even
    /// under concurrent callers.
    pub fn claim(&self, bytes: u64) -> bool {
        let mut current = self.consumed.load(Ordering::SeqCst);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                return false;
            };
            if next > self.max_bytes {
                return false;
            }
            match self
                .consumed
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }
}

impl Default for DownloadBudget {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_RUN_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wallet refuses the claim that would overrun, and never overruns
    /// under concurrent claims.
    #[test]
    fn claims_stop_at_the_limit_even_concurrently() {
        let budget = DownloadBudget::new(100);
        assert!(budget.claim(60));
        assert!(budget.claim(40));
        assert!(!budget.claim(1), "the next byte trips the budget");
        assert_eq!(budget.consumed(), 100);

        let shared = DownloadBudget::new(10_000);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let wallet = shared.clone();
                std::thread::spawn(move || {
                    let mut claimed = 0u64;
                    while wallet.claim(7) {
                        claimed += 7;
                    }
                    claimed
                })
            })
            .collect();
        let total: u64 = handles
            .into_iter()
            .map(|handle| handle.join().expect("join"))
            .sum();
        assert_eq!(total, shared.consumed(), "claims are atomic");
        assert!(total <= 10_000, "no overrun under concurrency");
    }
}
