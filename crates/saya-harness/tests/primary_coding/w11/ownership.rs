//! Apply-back must renew checkout authority before it records a mutation intent.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{ApplyBackBounds, ApplyBackError, apply_back};
use saya_harness::coding::worktree::OwnershipError;
use std::fs;

use super::apply_support::{accepted_for, assigned, trio};
use super::support_w11::{apply_input, child_delta, child_write, plan_index, state_dir};

#[tokio::test]
async fn moved_child_or_candidate_head_is_refused_before_user_writes() {
    for move_candidate in [false, true] {
        let label = if move_candidate {
            "w11-owner-c"
        } else {
            "w11-owner-a"
        };
        let t = trio(label).await;
        child_write(&t.a, "a/one.txt", b"child bytes\n");
        let token = CancellationToken::new();
        let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt"]), &token);
        let accepted = accepted_for(&t.a, &delta, &receipt);
        let part = if move_candidate { &t.candidate } else { &t.a };
        child_write(part, "ownership-marker.txt", b"new head\n");
        t.fixture.git_output(&part.root, &["add", "-A"]);
        t.fixture.git_output(
            &part.root,
            &[
                "-c",
                "user.name=saya",
                "-c",
                "user.email=saya@example.com",
                "commit",
                "-m",
                "move checkout head",
            ],
        );
        let before = t
            .fixture
            .record(t.candidate.verified.checkout_id())
            .await
            .unwrap();
        let plan = plan_index(&t);
        let state = state_dir(&t);
        let result = apply_back(
            &apply_input(&t, &plan, &state, &[accepted]),
            &token,
            ApplyBackBounds::default(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ApplyBackError::Ownership(OwnershipError::Git(_)))
        ));
        assert_eq!(
            fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap(),
            b"nested one bytes\n"
        );
        assert!(!state.join("coding/apply-back").exists());
        assert_eq!(
            before,
            t.fixture
                .record(t.candidate.verified.checkout_id())
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn foreign_baseline_or_plan_is_refused_before_intent() {
    let t = trio("w11-binding").await;
    let foreign = trio("w11-foreign").await;
    child_write(&t.a, "a/one.txt", b"child bytes\n");
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt"]), &token);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let plan = plan_index(&t);
    let foreign_plan = plan_index(&foreign);
    let state = state_dir(&t);
    let before = t
        .fixture
        .record(t.candidate.verified.checkout_id())
        .await
        .unwrap();
    for replace_baseline in [false, true] {
        let mut input = apply_input(&t, &plan, &state, &accepted);
        if replace_baseline {
            input.baseline = &foreign.a.snapshot;
        } else {
            input.plan = &foreign_plan;
        }
        assert!(matches!(
            apply_back(&input, &token, ApplyBackBounds::default()).await,
            Err(ApplyBackError::SnapshotMismatch)
        ));
        assert!(!state.join("coding/apply-back").exists());
        assert_eq!(
            fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap(),
            b"nested one bytes\n"
        );
        assert_eq!(
            before,
            t.fixture
                .record(t.candidate.verified.checkout_id())
                .await
                .unwrap()
        );
    }
}
