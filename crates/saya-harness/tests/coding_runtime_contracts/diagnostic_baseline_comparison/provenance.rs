use super::{
    fixture_helpers::{invocation, run},
    support,
};
use saya_agent::CancellationToken;
use saya_harness::coding::{
    checks::{
        CheckExecutionError, CheckExecutionInput, CheckInvocation, DiagnosticEvidenceError,
        execute_check_with_diagnostics,
    },
    commands::CommandOwner,
};
use saya_types::{CheckDiagnosticFormat, Sha256Digest};
use std::num::NonZeroU64;

#[tokio::test]
async fn recorded_policy_and_epoch_must_match_even_with_identical_source() {
    let fixture = support::diagnostic_fixture("c04-policy").await;
    let owner = CommandOwner::new(1).unwrap();
    let invocation = invocation(CheckDiagnosticFormat::RustcJson, false, None);
    let original = run(&fixture, &owner, &invocation).await;
    let ownership = fixture.ownership();
    let token = CancellationToken::new();
    let baseline = original.baseline(&ownership, &token).await.unwrap();
    let view = fixture.authority();
    for (policy, epoch) in [
        (Sha256Digest::parse(&"b".repeat(64)).unwrap(), 0),
        (view.policy_digest().clone(), 1),
    ] {
        let authority = fixture.authority_with_identity(
            view.repository_id().clone(),
            view.snapshot_id().clone(),
            policy,
            epoch,
        );
        let current = execute_check_with_diagnostics(CheckExecutionInput {
            ledger: fixture.ledger(),
            ownership: &ownership,
            authority: &authority,
            invocation: &invocation,
            owner: &owner,
            cancellation: &token,
            program_dir: fixture.commands.program_dir(),
            output_root: &fixture.output_root,
            resolver: fixture.resolver(),
        })
        .await
        .unwrap();
        assert!(current.diagnostics().is_ok());
        assert!(matches!(
            current
                .compare_baseline(&baseline, &ownership, &token)
                .await,
            Err(DiagnosticEvidenceError::Incomparable)
        ));
    }
}

#[tokio::test]
async fn missing_format_is_refused_before_command_admission() {
    let fixture = support::diagnostic_fixture("c04-missing-format").await;
    let owner = CommandOwner::new(1).unwrap();
    let invocation = CheckInvocation::new(
        "profile".into(),
        "check".into(),
        "not-authorized".into(),
        vec![],
        NonZeroU64::new(10).unwrap(),
        None,
        "confined".into(),
    )
    .unwrap();
    let ownership = fixture.ownership();
    let token = CancellationToken::new();
    let result = execute_check_with_diagnostics(CheckExecutionInput {
        ledger: fixture.ledger(),
        ownership: &ownership,
        authority: fixture.authority(),
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
        Err(CheckExecutionError::DiagnosticFormatRequired)
    ));
    // A subsequent admitted run must still acquire the single process slot.
    let valid = super::fixture_helpers::invocation(CheckDiagnosticFormat::RustcJson, false, None);
    assert!(
        run(&fixture, &owner, &valid)
            .await
            .receipt()
            .is_successful()
    );
}
