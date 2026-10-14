//! W11 support: the apply-state directory, the capture-time plan index, the
//! child-edit helpers, the fixed `ApplyBackInput` wiring, and journal
//! probes — all built on the W08 trio fixture.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, ApplyBackInput, AssignedPaths, ChangeReceipt, CheckoutDelta,
    DEFAULT_DELTA_DEADLINE, PlanIndex, apply_back_latest_dir, compute_delta, evaluate_receipt,
};
use saya_types::JobId;
use std::path::{Path, PathBuf};

use super::apply_support::{Part, Trio};

/// The harness-owned apply-back state directory for one test.
pub(super) fn state_dir(t: &Trio) -> PathBuf {
    let dir = t.fixture.temp.path().join("apply-state");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The capture-time plan facts parsed from the fixture's canonical plan
/// bytes and their manifest digest.
pub(super) fn plan_index(t: &Trio) -> PlanIndex {
    let bytes = t.fixture.plan.canonical_bytes();
    let sha = t.fixture.plan.manifest_sha256();
    PlanIndex::parse(sha, bytes).unwrap()
}

/// One child edit: writes (creating parents of) one child path.
pub(super) fn child_write(part: &Part, path: &str, bytes: &[u8]) {
    let full = part.root.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(full, bytes).unwrap();
}

/// One child deletion.
pub(super) fn child_remove(part: &Part, path: &str) {
    std::fs::remove_file(part.root.join(path)).unwrap();
}

/// The in-scope delta and its receipt after all of one child's edits.
pub(super) fn child_delta(
    part: &Part,
    assignment: &AssignedPaths,
    token: &CancellationToken,
) -> (CheckoutDelta, ChangeReceipt) {
    let delta = compute_delta(
        &part.verified,
        &part.snapshot,
        token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    let receipt = evaluate_receipt(&part.verified, &delta, assignment, u64::MAX);
    (delta, receipt)
}

/// The fixed apply-back input wiring over the trio fixture.
pub(super) fn apply_input<'a>(
    t: &'a Trio,
    plan: &'a PlanIndex,
    state: &'a Path,
    accepted: &'a [AcceptedDelta<'a>],
) -> ApplyBackInput<'a> {
    ApplyBackInput {
        store: &t.fixture.store,
        sequencer: &t.fixture.sequencer,
        job_id: &t.fixture.job,
        agent_id: None,
        repository: &t.fixture.repository,
        service: &t.fixture.service,
        candidate: &t.candidate.verified,
        baseline: &t.a.snapshot,
        plan,
        accepted,
        artifacts: &t.fixture.artifacts,
        state_dir: state,
    }
}

/// The latest apply-back journal directory for the fixture's job.
pub(super) fn journal_dir(state: &Path, job: &JobId) -> PathBuf {
    apply_back_latest_dir(state, job).expect("the apply-back journal directory is recorded")
}

/// The number of journal lines carrying one `"event"` kind.
pub(super) fn journal_event_count(dir: &Path, event: &str) -> usize {
    let text = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap();
    text.lines()
        .filter(|line| line.contains(&format!("\"event\":\"{event}\"")))
        .count()
}
