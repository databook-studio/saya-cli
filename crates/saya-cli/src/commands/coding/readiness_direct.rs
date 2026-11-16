//! Advisory readiness for a direct-mode coordinator: facts about the root
//! working checkout W, the live children and the current candidate generation,
//! plus one `suggestion`. Delegation is the coordinator's decision, so there
//! is no forced next action — `next_action` stays unset — and `candidate_ready`
//! comes only from a prepared receipt for the current generation.
//!
//! The facts are bounded, redacted and carry no authority: which paths W
//! changed and how, and which children ended failed or uncertain with the class
//! their own completion recorded.

use super::{
    NativeCodingJob, continuation::is_terminal, prepare::SourceGeneration,
    prepare_receipt::PreparedReceipt, readiness::CandidateReadiness,
    readiness_cache::ReadinessFacts,
};
use saya_harness::coding::integration::DeltaKind;
use saya_store::{CodingChildRecord, CodingCompletionPage, StoreError};
use saya_types::{AgentId, CodingFailureClass, JobState};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    num::NonZeroU64,
    sync::Arc,
};

/// Most live children named by one advisory view; the rest are counted in
/// `live_children_omitted`.
const MAX_LIVE_CHILDREN: usize = 32;

/// Most changed paths one advisory view names; the rest are counted by
/// `changed_paths_truncated`.
const MAX_CHANGED_PATHS: usize = 64;

/// Most failed or uncertain children one advisory view names.
const MAX_CHILD_FAILURES: usize = 8;

/// Most pages of the job's completion history one advisory read walks to find a
/// child's failure class, one page of the store's own maximum size. A job's
/// history is store-bounded (1,024 completions), but a cache miss — one per root
/// write or command — must not pay for all of it, so the walk stops
/// here. The store reads completions oldest-first only, so a history longer
/// than this cap is read from its START: the walk reports `history_truncated`,
/// because a later completion may carry a later class.
const MAX_COMPLETION_HISTORY_PAGES: usize = 16;
const COMPLETION_HISTORY_PAGE: usize = saya_store::MAX_COMPLETION_PAGE_SIZE;

/// The advisory direct-mode readiness. A read-only view: W is re-verified from
/// its private record without creating one, and a record that cannot be read
/// degrades to unknown facts instead of a forced step. The expensive facts
/// come from the cache; the live children never do, because they move without
/// touching W.
pub(super) async fn direct(job: &NativeCodingJob) -> CandidateReadiness {
    let live = live_children(job).await;
    let facts = job.readiness_facts().await;
    // A live child can still change W, so a receipt can go stale: ready is
    // reported only when nothing is running.
    let ready = facts.receipt_ready && live.is_empty();
    // A deferred job (headless `start`) never runs the end-of-run preparation,
    // and a job that already ended has had its one chance. Only a live,
    // non-deferred run still prepares; every other changed checkout is
    // prepared on demand by `saya job review`.
    let deferred = job
        .deferred_preparation
        .load(std::sync::atomic::Ordering::Acquire);
    let running = job
        .application
        .store
        .get_coding_job(job.job_id())
        .await
        .ok()
        .flatten()
        .is_some_and(|record| !is_terminal(record.state));
    let suggestion: std::borrow::Cow<'_, str> = if !live.is_empty() {
        "wait for the helpers call to return".into()
    } else if facts.w_changed && !facts.receipt_ready {
        if deferred || !running {
            format!(
                "changes are in the workspace; run `saya job review {}` to prepare a candidate",
                job.job_id()
            )
            .into()
        } else {
            "the owner prepares the candidate at the end of the run".into()
        }
    } else {
        "none".into()
    };
    let generation = facts.generation.as_ref();
    let mut view = json!({
        "state": if !live.is_empty() {
            "children_active"
        } else if ready {
            "candidate_ready"
        } else if facts.w_changed {
            "changed"
        } else if generation.is_some() {
            "working"
        } else {
            "unknown"
        },
        "candidate_ready": ready,
        "candidate": generation.map(|g| json!(g.candidate)).unwrap_or(Value::Null),
        "source_generation_sha256": generation
            .map(|g| json!(g.digest))
            .unwrap_or(Value::Null),
        "live_children": live.iter().take(MAX_LIVE_CHILDREN).collect::<Vec<_>>(),
        "live_children_omitted": live.len().saturating_sub(MAX_LIVE_CHILDREN),
        "w_changed": facts.w_changed,
        "changed_paths": facts.changed_paths,
        "changed_paths_truncated": facts.changed_paths_truncated,
        "child_failures": facts.child_failures,
        // Whether the walk that produced `child_failures` read the whole
        // completion history: when it did not, a class is the latest the walk
        // SAW, and a later completion may have recorded a later one.
        "history_truncated": facts.history_truncated,
        "suggestion": suggestion,
    });
    // Why the owner's end-of-run preparation produced no ready candidate: a
    // closed name and one fixed sentence, and nothing else. Absent when the
    // owner prepared a ready candidate, prepared nothing, or never prepared.
    if let Some(refusal) = job.preparation_refusal() {
        view["preparation"] = json!({
            "reason": refusal.reason(),
            "detail": refusal.detail(),
        });
    }
    CandidateReadiness::new(view)
}

impl NativeCodingJob {
    /// W's delta, the current source generation and its receipt readiness —
    /// the facts a readiness view needs and the whole-checkout scan it costs.
    /// Always computed from the checkout, never read from the cache: the
    /// advisory cache is the only caller, on a miss, and preparation, review
    /// and apply read the checkout through their own paths. `children` is the
    /// listing the caller already read, so the facts and the cache key
    /// describe one snapshot.
    pub(super) async fn compute_readiness_facts(
        &self,
        children: &[CodingChildRecord],
    ) -> ReadinessFacts {
        // Verify W once: the delta and the generation's evidence binding are
        // both read from the same verified checkout, so a status read pays one
        // inventory instead of two. The start flow's own proof — the completed
        // create or the resume verification — is spent here while W's epoch is
        // unchanged, so the first readiness view does not re-walk a checkout
        // nothing has touched yet.
        let verified = match self.root_checkout_proof() {
            Some(verified) => Some(verified),
            None => self
                .verified_root_checkout()
                .await
                .ok()
                .flatten()
                .map(Arc::new),
        };
        let ignored = match &verified {
            Some(verified) => self.checkout_ignored_paths(verified).await.ok(),
            None => None,
        };
        let changes = match (&verified, &ignored) {
            (Some(verified), Some(ignored)) if !self.request.write_paths.is_empty() => {
                super::in_place_changes::job_changes(
                    &self.application.artifacts,
                    &self.records,
                    verified,
                    ignored,
                    &self.cancellation,
                )
                .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let generation = verified
            .as_ref()
            .zip(ignored.as_ref())
            .and_then(|(verified, ignored)| self.source_generation_from(verified, ignored).ok());
        // W's path and base are fixed for the job's checkout; the outcome's
        // commit walk reads them from here rather than verifying W again.
        let checkout = verified.as_ref().and_then(|verified| {
            verified
                .base_commit()
                .cloned()
                .map(|base| (verified.path().clone(), base))
        });
        let receipt = match &generation {
            Some(generation) => self.prepared_receipt(generation).await,
            None => None,
        };
        let receipt_ready = receipt
            .as_ref()
            .is_some_and(PreparedReceipt::candidate_ready);
        let (changed_paths, changed_paths_truncated) = changed_paths(&changes);
        let (child_failures, history_truncated) = self.child_failures(children).await;
        ReadinessFacts {
            w_changed: !changes.is_empty(),
            generation,
            receipt_ready,
            changed_paths,
            changed_paths_truncated,
            child_failures,
            history_truncated,
            checkout,
        }
    }

    /// The current generation's prepared receipt, or `None` when it has none or
    /// cannot be read. The one read behind `receipt_ready`.
    async fn prepared_receipt(&self, generation: &SourceGeneration) -> Option<PreparedReceipt> {
        let path = self.prepared_receipt_path(&generation.digest);
        if !path.exists() {
            return None;
        }
        self.read_prepared_receipt(&path, generation).await.ok()
    }

    /// The children that ended failed or uncertain, each with the class its own
    /// latest durable completion recorded — `null` when it recorded none — and
    /// whether the walk that found those classes reached the end of the
    /// completion history. A child that fails under `delegate` without an await
    /// has no other surface.
    async fn child_failures(&self, children: &[CodingChildRecord]) -> (Vec<Value>, bool) {
        let failures = children
            .iter()
            .filter(|child| matches!(child.state, JobState::Failed | JobState::Uncertain))
            .take(MAX_CHILD_FAILURES)
            .collect::<Vec<_>>();
        let wanted = failures
            .iter()
            .map(|child| child.agent_id.clone())
            .collect::<BTreeSet<_>>();
        let walked = self.completion_failure_classes(&wanted).await;
        let failures = failures
            .into_iter()
            .map(|child| {
                json!({
                    "agent_id": child.agent_id,
                    "outcome": child.state,
                    "failure_class": walked
                        .classes
                        .get(&child.agent_id)
                        .copied()
                        .flatten()
                        .map(|class| json!(class.as_str()))
                        .unwrap_or(Value::Null),
                })
            })
            .collect();
        (failures, walked.history_truncated)
    }

    /// Each wanted child's latest durable completion's failure class, over at
    /// most `MAX_COMPLETION_HISTORY_PAGES` pages of the job's completion
    /// history. Only a failed or uncertain completion records a class, so every
    /// other outcome maps to `None`.
    async fn completion_failure_classes(
        &self,
        wanted: &BTreeSet<AgentId>,
    ) -> CompletionFailureClasses {
        let store = &self.application.store;
        let job_id = self.job_id();
        latest_failure_classes(wanted, move |cursor| async move {
            store
                .list_coding_completion_history(job_id, cursor, COMPLETION_HISTORY_PAGE)
                .await
        })
        .await
    }
}

/// What one walk of the completion history found: the failure class each
/// `wanted` agent's latest READ completion recorded, and whether the walk
/// stopped short of the end of the history.
pub(crate) struct CompletionFailureClasses {
    pub(crate) classes: BTreeMap<AgentId, Option<CodingFailureClass>>,
    /// Whether the walk did not reach the end of the history: it hit its page
    /// cap, or a page could not be read. The classes are then the latest ones
    /// the walk SAW — a later completion may carry a later class, so a view
    /// built from them must say so rather than claim the latest completion won.
    pub(crate) history_truncated: bool,
}

/// The failure class each `wanted` agent's latest completion recorded, or `None`
/// when it recorded none, and whether the walk reached the end of the history.
/// The walk is bounded to `MAX_COMPLETION_HISTORY_PAGES` pages and keeps an
/// entry only for a wanted agent, so the map stays O(children) whatever the
/// history holds; later pages overwrite earlier ones, so the latest completion
/// the walk read wins. The store reads completions oldest-first only, so a
/// history longer than the cap is read from its start and reported truncated. A
/// page that cannot be read ends the walk with what it has, which can only miss
/// a class, never invent one — and also reports truncated, because the pages it
/// never read may hold a later class.
async fn latest_failure_classes<F, Fut>(
    wanted: &BTreeSet<AgentId>,
    mut fetch: F,
) -> CompletionFailureClasses
where
    F: FnMut(Option<NonZeroU64>) -> Fut,
    Fut: Future<Output = Result<CodingCompletionPage, StoreError>>,
{
    let mut classes = BTreeMap::new();
    let mut cursor = None;
    // Until the walk reads a page with no next cursor, the history may hold
    // more: an unread page and a page that failed to read both leave it set.
    let mut history_truncated = true;
    for _ in 0..MAX_COMPLETION_HISTORY_PAGES {
        let Ok(page) = fetch(cursor).await else {
            break;
        };
        for item in &page.items {
            let agent = item.result.agent_id();
            if wanted.contains(agent) {
                classes.insert(agent.clone(), item.result.failure_class());
            }
        }
        let Some(next) = page.next_cursor else {
            history_truncated = false;
            break;
        };
        cursor = Some(next);
    }
    CompletionFailureClasses {
        classes,
        history_truncated,
    }
}

/// W's delta as the view names it: at most `MAX_CHANGED_PATHS` entries in the
/// delta's own (path-sorted) order, plus whether the list dropped any. Only the
/// path and the kind leave — never the bytes or the content.
pub(super) fn changed_paths(changes: &[super::root_checkout::RootChange]) -> (Vec<Value>, bool) {
    let view = changes
        .iter()
        .take(MAX_CHANGED_PATHS)
        .map(|change| {
            json!({
                "path": change.path,
                "kind": change_kind(change.kind),
            })
        })
        .collect();
    (view, changes.len() > MAX_CHANGED_PATHS)
}

/// How one delta kind reads to the model and the operator.
fn change_kind(kind: DeltaKind) -> &'static str {
    match kind {
        DeltaKind::Added => "created",
        DeltaKind::Modified => "modified",
        DeltaKind::Deleted => "deleted",
    }
}

/// The children that are not yet terminal: live delegation the coordinator may
/// still be waiting on. A record that cannot be read is not a live child.
async fn live_children(job: &NativeCodingJob) -> Vec<Value> {
    let Ok(children) = job
        .application
        .store
        .list_coding_children(job.job_id(), None, 100)
        .await
    else {
        return Vec::new();
    };
    children
        .items
        .iter()
        .filter(|child| !is_terminal(child.state))
        .map(|child| json!({"agent_id": child.agent_id, "state": child.state}))
        .collect()
}

#[cfg(all(test, unix))]
#[path = "readiness_direct_tests.rs"]
mod tests;
