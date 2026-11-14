//! The advisory direct-mode readiness cache.
//!
//! W's delta and the current source generation's evidence binding cost a whole
//! scan of the root working checkout on every call, and a model polls
//! `coding_status` often. Those facts are cached and recomputed when the key
//! moves — that is, when something that can change W has happened.
//!
//! ONLY the advisory readiness may use this cache: `coding_prepare_candidate`,
//! the owner preparation at the end of a run, review and apply always
//! recompute from the checkout.

use super::NativeCodingJob;
use super::context;
use super::prepare::SourceGeneration;
use super::prepare_receipt::MAX_PREPARED_RECEIPT_BYTES;
use saya_harness::coding::worktree::VerifiedCheckout;
use saya_store::CodingChildRecord;
use serde_json::Value;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

/// The conservative cache key. Every term moves when something that can change
/// the facts has happened: the root's mutating-ATTEMPT epoch (a refused or
/// failed write, edit, delete or program run), the receipt of the cached
/// generation (the owner preparing — or rewriting — the candidate these facts
/// describe), and the children's states — a child that fails moves no file and
/// no receipt, so its state is the only term that can carry it.
///
/// The receipt term is the receipt's own bytes, not merely its presence: a
/// receipt that is rewritten in place — a candidate the owner does not report
/// ready — leaves the path occupied, so presence alone would serve the stale
/// `ready` it replaced.
#[derive(Clone)]
struct ReadinessKey {
    epoch: u64,
    receipt: u64,
    children: u64,
}

/// The expensive part of the advisory readiness: whether W changed, the
/// current source generation, whether that generation's receipt is ready, and
/// the bounded, redacted facts the view names.
pub(super) struct ReadinessFacts {
    pub(super) w_changed: bool,
    pub(super) generation: Option<SourceGeneration>,
    pub(super) receipt_ready: bool,
    pub(super) changed_paths: Vec<Value>,
    pub(super) changed_paths_truncated: bool,
    pub(super) child_failures: Vec<Value>,
    /// Whether the completion-history walk that produced `child_failures`
    /// reached the end of the history. When it did not, the classes are the
    /// latest the walk read, and a later completion may carry a later one.
    pub(super) history_truncated: bool,
    /// W's path and base commit at the last verification, for the outcome's
    /// commit walk. Both are fixed for the job's checkout, so a cached pair is
    /// as true as a fresh verification.
    pub(super) checkout: Option<(saya_types::ManagedCheckoutPath, saya_types::GitObjectId)>,
}

/// The cached facts, the key they were computed under, and how many
/// computations were made (the lifecycle tests' counter).
#[derive(Default)]
pub(super) struct ReadinessCache {
    entry: Mutex<Option<(ReadinessKey, Arc<ReadinessFacts>)>>,
    computations: AtomicU64,
}

impl ReadinessCache {
    /// The cached entry, if any; the caller validates it with `still_current`.
    fn peek(&self) -> Option<(ReadinessKey, Arc<ReadinessFacts>)> {
        self.entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn store(&self, key: ReadinessKey, facts: Arc<ReadinessFacts>) {
        *self
            .entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((key, facts));
    }

    fn note_computation(&self) {
        self.computations.fetch_add(1, Ordering::AcqRel);
    }

    fn computations(&self) -> u64 {
        self.computations.load(Ordering::Acquire)
    }
}

impl NativeCodingJob {
    /// Bumps W's invalidation epoch: every root tool ATTEMPT that could have
    /// changed it, whatever its outcome. A refused or failed call can still have
    /// changed W, so it invalidates the advisory facts too; only a successful
    /// workspace mutation additionally counts as progress.
    pub(crate) fn note_root_w_attempt(&self) {
        self.root_w_epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// Seeds the working-checkout proof this start flow just produced (a
    /// completed create or a resume verification), keyed by W's current
    /// invalidation epoch. This is a use-time proof, not standing authority:
    /// the first root tool attempt moves the epoch and retires it.
    pub(crate) fn seed_root_checkout(&self, verified: Option<VerifiedCheckout>) {
        let epoch = self.root_w_epoch.load(Ordering::Acquire);
        let mut seed = self
            .root_checkout_seed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *seed = verified.map(|verified| (epoch, Arc::new(verified)));
    }

    /// The seeded working-checkout proof, only while W's invalidation epoch
    /// is unchanged since it was taken. `None` after any root tool attempt,
    /// so the advisory readiness then verifies W live.
    pub(crate) fn root_checkout_proof(&self) -> Option<Arc<VerifiedCheckout>> {
        let epoch = self.root_w_epoch.load(Ordering::Acquire);
        let seed = self
            .root_checkout_seed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match seed.as_ref() {
            Some((seeded, verified)) if *seeded == epoch => Some(Arc::clone(verified)),
            _ => None,
        }
    }

    /// How many times the advisory readiness recomputed W's facts: a cache hit
    /// adds nothing. The lifecycle tests read it to assert that repeated
    /// status reads compute once.
    pub fn readiness_computations(&self) -> u64 {
        self.readiness_cache.computations()
    }

    /// Whether this run's readiness facts were precomputed off the critical
    /// path. False when the job never started the precompute (no write path,
    /// hooks or MCP configured, or no assistant text was seen).
    pub fn readiness_precomputed(&self) -> bool {
        self.readiness_precompute
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|precompute| precompute.precomputed())
    }

    /// Starts the off-critical-path readiness computation. The first assistant
    /// text token is the signal: a turn that streams text may be the final one,
    /// and the cache key retires the result if a later call changes W.
    pub(super) fn start_readiness_precompute(&self) {
        let precompute = self
            .readiness_precompute
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(precompute) = precompute {
            precompute.start();
        }
    }

    /// Waits for a started precompute so the outcome reuses its facts instead
    /// of paying the scan again. A precompute that never started returns at
    /// once.
    pub(super) async fn settle_readiness_precompute(&self) {
        let precompute = self
            .readiness_precompute
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(precompute) = precompute {
            precompute.settle().await;
        }
    }

    /// Plans the run's advisory precompute. It may overlap the model loop only
    /// when nothing outside the cache key can change W: a hook or an MCP tool
    /// writes the checkout without bumping the root's epoch, so with either
    /// configured the outcome must compute the facts itself.
    pub(super) fn plan_readiness_precompute(self: &Arc<Self>, hooks_configured: bool) {
        let mut held = self
            .readiness_precompute
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *held = None;
        if !self.request.write_paths.is_empty()
            && !hooks_configured
            && self.mcp_servers().is_empty()
        {
            *held = Some(ReadinessPrecompute::new(self));
        }
    }

    /// The advisory facts, recomputed only when the key moved. The children are
    /// read once and drive both the key's signature and the facts, so the
    /// cached entry always describes exactly the children it was keyed on.
    pub(super) async fn readiness_facts(&self) -> Arc<ReadinessFacts> {
        let epoch = self.root_w_epoch.load(Ordering::Acquire);
        let children = self.child_records().await;
        let children_signature = children_signature(&children);
        if let Some((key, facts)) = self.readiness_cache.peek()
            && still_current(&key, epoch, self.receipt_digest(&facts), children_signature)
        {
            return facts;
        }
        self.readiness_cache.note_computation();
        let facts = Arc::new(self.compute_readiness_facts(&children).await);
        let key = ReadinessKey {
            epoch,
            receipt: self.receipt_digest(&facts),
            children: children_signature,
        };
        self.readiness_cache.store(key, Arc::clone(&facts));
        facts
    }

    /// The job's children as the store holds them. A store that cannot be read
    /// reports none, which can only miss (never wrongly hit) a state that moved.
    async fn child_records(&self) -> Vec<CodingChildRecord> {
        self.application
            .store
            .list_coding_children(self.job_id(), None, 100)
            .await
            .map(|page| page.items)
            .unwrap_or_default()
    }

    /// The prepared receipt of the cached generation as the key holds it: a
    /// digest of its bytes, or 0 when there is none. A generation with no
    /// receipt — and a job with no generation yet — has none, so preparing one
    /// moves the key; and because the digest covers the bytes, so does
    /// rewriting one that is already there.
    fn receipt_digest(&self, facts: &ReadinessFacts) -> u64 {
        facts
            .generation
            .as_ref()
            .map(|generation| self.prepared_receipt_path(&generation.digest))
            .map_or(0, |path| receipt_digest(&path))
    }
}

/// A cheap fingerprint of the prepared receipt at `path`, or 0 when it is
/// absent or cannot be read. Only equality against the cached key matters, but
/// it must move whenever the bytes do: the owner rewrites a receipt in place
/// when a candidate stops being ready, and a cached `ready`
/// must not outlive the receipt it was read from. An unreadable receipt reports
/// 0, which can only miss (never wrongly hit) the entry it replaced.
fn receipt_digest(path: &std::path::Path) -> u64 {
    let Ok(bytes) = context::read(path, MAX_PREPARED_RECEIPT_BYTES) else {
        return 0;
    };
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    // Folded to an odd value so a present receipt can never read as absent.
    hasher.finish() | 1
}

/// Whether the cached key still describes the job: the same epoch, the same
/// receipt bytes the cached generation had, and the same children states.
fn still_current(key: &ReadinessKey, epoch: u64, receipt: u64, children: u64) -> bool {
    key.epoch == epoch && key.receipt == receipt && key.children == children
}

/// A cheap signature of the children's states, in the store's own (agent-id
/// sorted) order. Only the state and the recorded outcome matter: a child that
/// fails without an await changes nothing else the facts describe.
fn children_signature(children: &[CodingChildRecord]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for child in children {
        child.agent_id.hash(&mut hasher);
        format!("{:?}", child.state).hash(&mut hasher);
        format!("{:?}", child.result_outcome).hash(&mut hasher);
    }
    hasher.finish()
}

/// The advisory readiness computed while the model is still streaming. The
/// whole-checkout scan blocks, and the coding runtime is single-threaded, so
/// the computation runs on the blocking pool: a `tokio::spawn` would stall the
/// very stream it is meant to overlap with.
///
/// The result is the cache entry `readiness_facts` would have computed, keyed
/// the same way, so the outcome's view is identical whether or not the
/// precompute ran. If a later tool call moves W, the epoch or children
/// signature retires the entry and the outcome recomputes.
pub(super) struct ReadinessPrecompute {
    job: Arc<NativeCodingJob>,
    started: AtomicBool,
    done: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ReadinessPrecompute {
    pub(super) fn new(job: &Arc<NativeCodingJob>) -> Arc<Self> {
        Arc::new(Self {
            job: Arc::clone(job),
            started: AtomicBool::new(false),
            done: AtomicBool::new(false),
            task: Mutex::new(None),
        })
    }

    /// Starts the computation exactly once. Called from the event sink, so it
    /// must not block: the blocking pool does the scan.
    pub(super) fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let runtime = tokio::runtime::Handle::current();
        let state = Arc::clone(self);
        let task = tokio::task::spawn_blocking(move || {
            runtime.block_on(state.job.readiness_facts());
            state.done.store(true, Ordering::Release);
        });
        let mut slot = self
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(task);
    }

    /// Waits for the started computation. The facts are already in the cache;
    /// the outcome's own call then reads them instead of recomputing.
    pub(super) async fn settle(&self) {
        let task = self
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    pub(super) fn precomputed(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}

#[cfg(all(test, unix))]
#[path = "readiness_cache_tests.rs"]
mod tests;
