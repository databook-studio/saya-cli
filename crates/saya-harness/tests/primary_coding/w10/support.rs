//! W10 support: the capture-time plan index, the preflight input, and the
//! original-repo state probe, all wired from the W08 trio fixture.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, ApplyBackPreflight, AssignedPaths, ChangeReceipt, CheckoutDelta,
    DEFAULT_DELTA_DEADLINE, PlanIndex, PreflightBounds, PreflightInput, compute_delta,
    evaluate_receipt, preflight_apply_back,
};

use super::apply_support::{Part, Trio};

/// The capture-time plan facts parsed from the fixture's canonical plan
/// bytes and their manifest digest.
pub(super) fn plan_index(t: &Trio) -> PlanIndex {
    let bytes = t.fixture.plan.canonical_bytes();
    let sha = t.fixture.plan.manifest_sha256();
    PlanIndex::parse(sha, bytes).unwrap()
}

/// The preflight input for the original repository against child `a`'s
/// published baseline.
pub(super) fn input<'a>(
    t: &'a Trio,
    plan: &'a PlanIndex,
    accepted: &'a [AcceptedDelta<'a>],
) -> PreflightInput<'a> {
    PreflightInput {
        repository: &t.fixture.repository,
        service: &t.fixture.service,
        baseline: &t.a.snapshot,
        plan,
        accepted,
    }
}

/// The unrelated dirty file's bytes plus the original repo's porcelain
/// status, captured as one comparable state snapshot.
pub(super) fn original_state(t: &Trio) -> (Vec<u8>, String) {
    let bytes = std::fs::read(t.fixture.repo_root.join("unrelated.txt")).unwrap();
    let status = t
        .fixture
        .git_output(&t.fixture.repo_root, &["status", "--porcelain"]);
    (bytes, status)
}

/// One child edit plus its in-scope delta and accepted receipt.
pub(super) fn child_delta(
    part: &Part,
    path: &str,
    bytes: &[u8],
    assignment: &AssignedPaths,
    token: &CancellationToken,
) -> (CheckoutDelta, ChangeReceipt) {
    let full = part.root.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(full, bytes).unwrap();
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

/// One run of the apply-back preflight over `accepted`, with default bounds.
pub(super) async fn guard(
    t: &Trio,
    token: &CancellationToken,
    accepted: &[AcceptedDelta<'_>],
) -> ApplyBackPreflight {
    let plan = plan_index(t);
    let input = input(t, &plan, accepted);
    preflight_apply_back(&input, token, PreflightBounds::default())
        .await
        .unwrap()
}

/// User drift: writes (creating parents of) one original path.
pub(super) fn user_write(t: &Trio, path: &str, bytes: &[u8]) {
    let full = t.fixture.repo_root.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(full, bytes).unwrap();
}

/// User drift: deletes one original path.
pub(super) fn user_remove(t: &Trio, path: &str) {
    std::fs::remove_file(t.fixture.repo_root.join(path)).unwrap();
}

/// User drift: replaces one original file with a symlink.
pub(super) fn user_symlink(t: &Trio, path: &str, target: &str) {
    std::fs::remove_file(t.fixture.repo_root.join(path)).unwrap();
    std::os::unix::fs::symlink(target, t.fixture.repo_root.join(path)).unwrap();
}

/// User drift: stages one original path with a Read-free git add.
pub(super) fn user_stage(t: &Trio, path: &str) {
    t.fixture.git_output(&t.fixture.repo_root, &["add", path]);
}
