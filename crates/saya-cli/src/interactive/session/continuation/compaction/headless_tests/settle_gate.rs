//! The settle gate: a pending clarification or a cancelled/stopped turn is
//! never authority for automatic work, and the no-retry latch stays alone.

use super::super::settle_auto_compact;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use saya_agent::{ApprovalPolicy, TokenUsage};

use super::{answering_report, output, runtime_with_window};

/// (i) A pending clarification or a cancelled/stopped turn is never authority
/// for automatic work: `settle_auto_compact` returns `None` without running
/// the compaction — the runtime here is the unused one, so a None result
/// cannot involve any provider call — and the session is untouched.
#[test]
fn a_pending_clarification_or_a_stop_never_settles_automatic_work() {
    let root = std::env::temp_dir().join(format!("saya-c43a-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the gate-test root");
    let runtime = runtime_with_window(Some(1000));
    let mut state = SessionState::new("c43a-gate", None, "test-model");
    let session = &mut SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "c43a-gate",
        ApprovalPolicy::Ask,
        &root.join("session-dirs"),
    )
    .expect("the gate-test session acquires");
    // A turn that WOULD fire on the pure predicate (960 of 1000) but ended on
    // a clarification, and the same turn stopped instead of settling.
    let would_fire = output(vec![answering_report(960)], TokenUsage::new(960, 20), false);
    assert!(
        settle_auto_compact(&runtime, &mut state, session, &would_fire, true, false).is_none(),
        "(i) a pending clarification is not authority for automatic work"
    );
    assert!(
        settle_auto_compact(&runtime, &mut state, session, &would_fire, false, true).is_none(),
        "(i) a cancelled/stopped turn never compacts merely because output exists"
    );
    assert!(
        !state.auto_compact_failed,
        "(i) a gated settlement leaves the no-retry latch alone"
    );
    let _ = std::fs::remove_dir_all(&root);
}
