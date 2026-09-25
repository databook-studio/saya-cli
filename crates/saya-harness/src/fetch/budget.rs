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

    /// The carry arms the wallet with the spend the run's journal already
    /// records — the resume path. A wallet carried to 60 of its 100 refuses
    /// the claim that would overrun the *run's* budget, though a fresh
    /// wallet would have accepted it: the budget binds the run, not the
    /// invocation.
    #[test]
    fn the_carry_arms_the_wallet_with_the_spend_the_record_holds() {
        let budget = DownloadBudget::new(100);
        budget.carry(60);
        assert_eq!(budget.consumed(), 60);
        assert!(budget.claim(40), "the headroom the record leaves is real");
        assert_eq!(budget.consumed(), 100);
        assert!(
            !budget.claim(1),
            "the run's budget is spent, not a fresh one"
        );
        assert_eq!(budget.consumed(), 100, "the refused claim wrote nothing");
    }

    /// The carry records a level, never a refusal: a wallet carried exactly
    /// to its limit is not tripped — the exact-fill posture, the twin of the
    /// fresh-wallet test above. The next refused claim trips the latch the
    /// ordinary way; a figure the record holds must not stand in for a
    /// refusal that never happened, which is the threshold comparison the
    /// latch exists to prevent.
    #[test]
    fn the_carry_never_synthesizes_a_refusal() {
        let exact = DownloadBudget::new(100);
        exact.carry(100);
        assert_eq!(exact.consumed(), 100);
        assert!(
            !exact.tripped(),
            "a carried level is not a refusal — no claim was ever refused"
        );
        assert!(!exact.claim(1), "past the limit the claims refuse");
        assert!(exact.tripped(), "the refusal is the recorded event");

        // Headroom carries the same way: a wallet carried below its limit
        // still claims what fits — the resume-with-headroom path the latch
        // doc names — and the first refusal after the carry trips it.
        let headroom = DownloadBudget::new(100);
        headroom.carry(97);
        assert!(!headroom.tripped());
        assert!(headroom.claim(3));
        assert!(
            !headroom.tripped(),
            "the last byte claimed, nothing refused"
        );
        assert!(!headroom.claim(1), "the next claim refuses");
        assert!(headroom.tripped());
    }

    /// A wallet carried past its limit — the resume that declares a smaller
    /// budget than the record's spend — refuses every claim, and the first
    /// one trips the latch: the record binds, the wallet never runs
    /// overdrawn.
    #[test]
    fn a_wallet_carried_past_its_limit_refuses_every_claim() {
        let overdrawn = DownloadBudget::new(100);
        overdrawn.carry(150);
        assert_eq!(overdrawn.consumed(), 150);
        assert!(!overdrawn.tripped(), "the carry itself refuses nothing");
        assert!(
            !overdrawn.claim(1),
            "a wallet carried past its limit refuses its first claim"
        );
        assert!(overdrawn.tripped());
        assert!(!overdrawn.claim(0), "every later claim refuses too");
    }

    /// The seeding figure is the highest level the journal holds — levels
    /// are monotone, so the highest is the spend the record carries — and
    /// every other event folds in as nothing. A journal that holds no
    /// download record carries nothing, never a fabricated zero.
    #[test]
    fn carried_from_journal_takes_the_highest_recorded_level() {
        let carried = DownloadBudget::carried_from_journal(&[
            RunEvent::RunStarted,
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::Usage {
                endpoint: "orchestrator".into(),
                tokens: Some(5),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: saya_types::PauseReason::BudgetExhausted,
            },
            RunEvent::DownloadedBytes { bytes: 40 },
        ]);
        assert_eq!(
            carried, 97,
            "the highest recorded level is the spend, never a lower one"
        );
        assert_eq!(
            DownloadBudget::carried_from_journal(&[RunEvent::RunStarted]),
            0,
            "a journal that holds no download record carries nothing"
        );
    }
}
