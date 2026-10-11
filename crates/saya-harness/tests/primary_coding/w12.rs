#![cfg(unix)]
#[path = "cleanup_support.rs"]
mod cleanup_support;
#[allow(dead_code)]
#[path = "w05/create_support.rs"]
mod create_support;
#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
use saya_agent::CancellationToken;
use saya_harness::coding::{
    budget::JobBudgetLedger,
    worktree::{CleanupError, CleanupInput, cleanup_managed_checkout, create_managed_checkout},
};
use saya_types::{ManagedCheckoutId, ManagedCheckoutLifecycle};

#[tokio::test]
async fn cleanup_preserves_dirty_work_and_removes_only_clean_recoverable_checkout() {
    for dirty in [true, false] {
        let fixture =
            cleanup_support::clean_fixture(if dirty { "w12-dirty" } else { "w12-clean" }).await;
        fixture.git_output(&fixture.repo_root, &["add", "tracked.txt"]);
        let original_index = std::fs::read(fixture.repo_root.join(".git/index")).unwrap();
        let original_content = std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap();
        let token = CancellationToken::new();
        let id = ManagedCheckoutId::parse("checkout").unwrap();
        let created = create_managed_checkout(&fixture.input(&id), &token)
            .await
            .unwrap();
        let path = std::path::PathBuf::from(created.path().as_str());
        if dirty {
            std::fs::write(path.join("precious.txt"), "preserve me").unwrap();
        }
        let ledger = JobBudgetLedger::new(
            fixture.store.clone(),
            fixture.job.clone(),
            create_support::limits(),
        )
        .await
        .unwrap();
        let quiescence = ledger.try_quiescent().await.unwrap();
        let sequencer = ledger.sequencer();
        let input = CleanupInput {
            ownership: cleanup_support::ownership(&fixture, &id),
            sequencer: &sequencer,
            quiescence: &quiescence,
        };
        assert!(matches!(
            saya_harness::coding::worktree::ensure_retention_capacity(
                &fixture.store,
                &fixture.service,
                &fixture.job,
                0,
                1,
                &token
            )
            .await,
            Err(CleanupError::RetentionBudgetExceeded)
        ));
        assert!(path.exists());
        let result = cleanup_managed_checkout(&input, &token).await;
        if dirty {
            assert!(matches!(result, Err(CleanupError::RetainWork)));
            assert_eq!(
                std::fs::read_to_string(path.join("precious.txt")).unwrap(),
                "preserve me"
            );
        } else {
            result.unwrap();
            assert!(!path.exists());
            assert_eq!(
                fixture.record(&id).await.unwrap().current_lifecycle,
                ManagedCheckoutLifecycle::Removed
            );
        }
        assert_eq!(
            std::fs::read(fixture.repo_root.join(".git/index")).unwrap(),
            original_index
        );
        assert_eq!(
            std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap(),
            original_content
        );
        assert!(
            fixture
                .artifacts
                .read_snapshot(fixture.plan.snapshot_id())
                .is_ok()
        );
    }
}
