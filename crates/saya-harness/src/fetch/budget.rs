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

use saya_types::RunEvent;

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

    /// Arms the wallet with the spend the run's journal already records —
    /// the resume path that makes the budget bind the **run**, not each
    /// invocation. Without it a resume armed a fresh wallet and N resumes
    /// cost N × the declared budget; with it the resumed run continues
    /// against the bytes already spent, and a run already past its limit
    /// (or over it — the record binds, the wallet refuses) is refused on
    /// its next download claim, the refusal tripping the latch exactly as
    /// a live refusal would. The carry is the spend **as the record holds
    /// it** — the highest level any journaled [`RunEvent::DownloadedBytes`]
    /// carries — and it never touches the trip latch: no figure the record
    /// holds is a refusal, and synthesizing one from a level would be the
    /// threshold comparison the latch exists to prevent.
    ///
    /// Called once per resume, before the resumed episodes run and while
    /// the run's single-writer lock is held — never concurrently with a
    /// claim. The wallet is shared by clone with the fetch-capable steps'
    /// executors, so arming it here arms every holder.
    pub fn carry(&self, bytes: u64) {
        let current = self.consumed.load(Ordering::SeqCst);
        self.consumed
            .store(current.saturating_add(bytes), Ordering::SeqCst);
    }

    /// The spend the run's journal records: the highest level any journaled
    /// [`RunEvent::DownloadedBytes`] carries. The levels are monotone —
    /// claims are never refunded — so the highest is the spend the record
    /// holds, and a journal that holds none carries nothing: the wallet the
    /// composition armed stays as it was built, never zeroed by a figure
    /// that was never recorded. Other events fold in as nothing.
    pub fn carried_from_journal(events: &[RunEvent]) -> u64 {
        events.iter().fold(0u64, |carried, event| match event {
            RunEvent::DownloadedBytes { bytes } => carried.max(*bytes),
            _ => carried,
        })
    }
}

impl Default for DownloadBudget {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_RUN_BYTES)
    }
}

#[cfg(test)]
mod tests;
