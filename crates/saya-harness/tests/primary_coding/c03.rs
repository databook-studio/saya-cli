//! C03a public check-execution proof (authored, not executed).

#[path = "c03/authority.rs"]
mod authority;
#[allow(dead_code)]
#[path = "c02_support.rs"]
mod c02_support;
#[path = "c03/support.rs"]
mod support;

use std::num::NonZeroU64;

use saya_agent::CancellationToken;
use saya_harness::coding::{
    checks::{CheckExecutionError, CheckExecutionInput, CheckInvocation, execute_check},
    commands::CommandOwner,
};
use saya_types::CodingCheckOutcome;

#[tokio::test]
async fn check_receipt_binds_real_exit_and_source() {
    let Some(fixture) = support::fixture("c03-real-exit").await else {
        return;
    };
    let owner = CommandOwner::new(1).unwrap();
    let denied = CheckInvocation::new(
        "c03-profile".into(),
        "ungranted-program".into(),
        "not-authorized".into(),
        Vec::new(),
        NonZeroU64::new(10).unwrap(),
        None,
        "confined".into(),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let denied_ownership = fixture.ownership();
    assert!(matches!(
        execute_check(CheckExecutionInput {
            ledger: fixture.ledger(),
            ownership: &denied_ownership,
            authority: fixture.authority(),
            invocation: &denied,
            owner: &owner,
            cancellation: &cancellation,
            program_dir: fixture.commands.program_dir(),
            output_root: &fixture.output_root,
            resolver: fixture.resolver(),
        })
        .await,
        Err(CheckExecutionError::ProgramDenied)
    ));

    let invocation = CheckInvocation::new(
        "c03-profile".into(),
        "c02-helper-check".into(),
        c02_support::HELPER.into(),
        Vec::new(),
        NonZeroU64::new(10).unwrap(),
        None,
        "confined".into(),
    )
    .unwrap();
    let ownership = fixture.ownership();
    let receipt = match execute_check(CheckExecutionInput {
        ledger: fixture.ledger(),
        ownership: &ownership,
        authority: fixture.authority(),
        invocation: &invocation,
        owner: &owner,
        cancellation: &cancellation,
        program_dir: fixture.commands.program_dir(),
        output_root: &fixture.output_root,
        resolver: fixture.resolver(),
    })
    .await
    {
        Ok(receipt) => receipt,
        Err(CheckExecutionError::ConfinementUnsupported) => {
            eprintln!("C03 skipped: host sandbox is not proven");
            return;
        }
        other => other.unwrap(),
    };

    assert_eq!(
        receipt.outcome(),
        CodingCheckOutcome::KnownExit { code: 112 }
    );
    assert!(
        !receipt.is_successful(),
        "a real nonzero exit stays failing"
    );
    assert_eq!(receipt.program(), c02_support::HELPER);
    assert_eq!(receipt.output().stream_names(), ["stdout", "stderr"]);
    assert!(receipt.output().stored_bytes() <= 2 * 64 * 1024);
    receipt
        .verify_at_use(&ownership, &cancellation)
        .await
        .unwrap();

    fixture.change_candidate_source();
    assert!(matches!(
        receipt.verify_at_use(&ownership, &cancellation).await,
        Err(CheckExecutionError::StaleReceipt)
    ));
    fixture.commit_original_candidate_source();
    assert!(matches!(
        receipt.verify_at_use(&ownership, &cancellation).await,
        Err(CheckExecutionError::Ownership(
            saya_harness::coding::worktree::OwnershipError::Git(_)
        ))
    ));
}

#[path = "c04.rs"]
mod diagnostics;
