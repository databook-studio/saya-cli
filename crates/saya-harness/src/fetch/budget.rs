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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Provisional default for a run's total download bytes: a full corpus
/// artifact set, bounded so one run cannot silently fill a disk. Provisional
/// until M5 measures real runs (U8) — the number exists so the default is
/// bounded, not because it is measured.
pub const DEFAULT_MAX_RUN_BYTES: u64 = 1024 * 1024 * 1024;

/// The per-run download wallet, shared across a run's downloads. The trip
/// latch rides the wallet so every clone sees it: a refusal — including the
/// arithmetic-overflow arm — is an event that happened, and the engine's
/// sink reads the event, not a threshold.
#[derive(Clone, Debug)]
pub struct DownloadBudget {
    max_bytes: u64,
    consumed: Arc<AtomicU64>,
    tripped: Arc<AtomicBool>,
}

impl DownloadBudget {
    /// A fresh wallet of `max_bytes` for one run.
    pub fn new(max_bytes: u64) -> Self {
        Self {
            max_bytes,
            consumed: Arc::new(AtomicU64::new(0)),
            tripped: Arc::new(AtomicBool::new(false)),
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

    /// Whether any claim was refused. This is a **latch, not a threshold**:
    /// the engine's pause check must not read `consumed() >= limit()`, which
    /// is wrong in both directions — a run that consumes exactly its budget
    /// with nothing refused must not pause, and a refusal can leave
    /// unclaimable headroom so `consumed` sits *below* `limit` at the moment
    /// of the trip. The latch records the event that actually happened: a
    /// claim was refused. Once set it never clears; cloning shares the flag
    /// with the wallet.
    pub fn tripped(&self) -> bool {
        self.tripped.load(Ordering::SeqCst)
    }

    /// Claims `bytes` for writing. `false` when the claim would exceed the
    /// limit — the caller must stop before writing (paused, not overrun),
    /// and the trip latch is set. Cloning shares the same wallet; claims
    /// never exceed the limit even under concurrent callers, and post-trip
    /// every subsequent claim fails the same way.
    pub fn claim(&self, bytes: u64) -> bool {
        let mut current = self.consumed.load(Ordering::SeqCst);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                self.trip();
                return false;
            };
            if next > self.max_bytes {
                self.trip();
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

    fn trip(&self) {
        self.tripped.store(true, Ordering::SeqCst);
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

    /// A refused claim sets the trip latch — the event the sink reads. This
    /// is the latch, not a threshold: the run consuming exactly its budget
    /// (the last claim succeeding) must NOT be tripped.
    #[test]
    fn an_exact_fill_trips_nothing_but_a_refusal_trips() {
        let exact = DownloadBudget::new(100);
        assert!(exact.claim(60));
        assert!(exact.claim(40), "the last byte claims successfully");
        assert_eq!(exact.consumed(), 100);
        assert!(
            !exact.tripped(),
            "consuming exactly the budget with nothing refused is not a trip: \
             the sink must not pause this run"
        );

        let refused = DownloadBudget::new(100);
        assert!(refused.claim(97));
        assert!(
            !refused.claim(7),
            "the chunk does not fit in the 3 bytes of headroom"
        );
        assert_eq!(
            refused.consumed(),
            97,
            "a refusal can leave unclaimable headroom — consumed sits BELOW the \
             limit at the moment of the trip, which is exactly why the latch \
             exists and a consumed>=limit threshold would miss it"
        );
        assert!(refused.tripped(), "the refusal is the recorded event");

        // The latch rides the clone: a clone of the tripped wallet is
        // tripped. The latch records, it does not gate — a claim that fits
        // the remaining headroom still succeeds (the resume-with-headroom
        // path), and the pause is the sink's decision on the next tick.
        let clone = refused.clone();
        assert!(clone.tripped());
        assert!(
            clone.claim(3),
            "a claim fitting the headroom still succeeds"
        );
        assert_eq!(clone.consumed(), 100);
        assert!(!clone.claim(1), "past the limit the claims refuse again");
    }

    /// The arithmetic-overflow arm is a refusal like any other: it trips the
    /// latch. A claim that cannot even be added must not read as
    /// "under the limit".
    #[test]
    fn the_overflow_arm_trips_the_latch() {
        let budget = DownloadBudget::new(100);
        assert!(budget.claim(50));
        assert!(
            !budget.claim(u64::MAX),
            "a claim that cannot even be added is refused, never wrapped"
        );
        assert!(
            budget.tripped(),
            "the overflow arm is a refusal like any other: it trips the latch"
        );
        assert_eq!(budget.consumed(), 50, "the overflow wrote nothing");
        assert!(
            !budget.claim(51),
            "the wallet's arithmetic still governs claims"
        );
    }
}
