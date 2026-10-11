//! A runner grant cannot cross repository or snapshot identity namespaces.

use super::{c02_support, support};
use saya_agent::CancellationToken;
use saya_harness::coding::{
    checks::{CheckExecutionError, CheckExecutionInput, CheckInvocation, execute_check},
    commands::CommandOwner,
};
use std::num::NonZeroU64;

#[tokio::test]
async fn unrelated_repository_or_snapshot_cannot_lend_a_program_grant() {
    let Some(fixture) = support::fixture("c03-identity").await else {
        return;
    };
    let invocation = CheckInvocation::new(
        "profile".into(),
        "identity-check".into(),
        c02_support::HELPER.into(),
        Vec::new(),
        NonZeroU64::new(10).unwrap(),
        None,
        "confined".into(),
    )
    .unwrap();
    let owner = CommandOwner::new(1).unwrap();
    let token = CancellationToken::new();
    let ownership = fixture.ownership();
    for snapshot in [false, true] {
        let authority = fixture.unrelated_authority(snapshot);
        let result = execute_check(CheckExecutionInput {
            ownership: &ownership,
            authority: &authority,
            invocation: &invocation,
            owner: &owner,
            cancellation: &token,
            program_dir: fixture.commands.program_dir(),
            output_root: &fixture.output_root,
            resolver: fixture.resolver(),
        })
        .await;
        assert!(matches!(
            result,
            Err(CheckExecutionError::AuthorityMismatch)
        ));
        assert!(!fixture.output_root.join("command-output").exists());
    }
}
