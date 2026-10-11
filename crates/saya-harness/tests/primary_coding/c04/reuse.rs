use super::{
    fixture_helpers::{invocation, run},
    support,
};
use saya_agent::CancellationToken;
use saya_harness::coding::{
    checks::{CheckExecutionError, DiagnosticEvidenceError},
    commands::CommandOwner,
};
use saya_types::CheckDiagnosticFormat;

#[tokio::test]
async fn baseline_mint_and_current_comparison_refuse_stale_source() {
    let fixture = support::diagnostic_fixture("c04-freshness").await;
    let owner = CommandOwner::new(1).unwrap();
    let invocation = invocation(CheckDiagnosticFormat::RustcJson, false, None);
    let token = CancellationToken::new();
    let ownership = fixture.ownership();
    let old = run(&fixture, &owner, &invocation).await;
    let baseline = old.baseline(&ownership, &token).await.unwrap();
    fixture.change_candidate_source();
    assert!(matches!(
        old.baseline(&ownership, &token).await,
        Err(DiagnosticEvidenceError::Execution(
            CheckExecutionError::StaleReceipt
        ))
    ));
    let current = run(&fixture, &owner, &invocation).await;
    fixture.set_candidate_source(b"changed again after current check\n");
    assert!(matches!(
        current
            .compare_baseline(&baseline, &ownership, &token)
            .await,
        Err(DiagnosticEvidenceError::Execution(
            CheckExecutionError::StaleReceipt
        ))
    ));
}

#[tokio::test]
async fn absent_toolchain_is_not_an_explicit_unknown_toolchain_baseline() {
    let fixture = support::diagnostic_fixture("c04-toolchain").await;
    let owner = CommandOwner::new(1).unwrap();
    let original = run(
        &fixture,
        &owner,
        &invocation(CheckDiagnosticFormat::RustcJson, false, None),
    )
    .await;
    let token = CancellationToken::new();
    let ownership = fixture.ownership();
    let baseline = original.baseline(&ownership, &token).await.unwrap();
    let current = run(
        &fixture,
        &owner,
        &invocation(CheckDiagnosticFormat::RustcJson, false, Some("unknown")),
    )
    .await;
    assert!(current.diagnostics().is_ok());
    assert_eq!(
        original.receipt().wire_receipt().toolchain(),
        current.receipt().wire_receipt().toolchain()
    );
    assert!(matches!(
        current
            .compare_baseline(&baseline, &ownership, &token)
            .await,
        Err(DiagnosticEvidenceError::Incomparable)
    ));
}
