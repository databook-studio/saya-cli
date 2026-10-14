//! C04 public producer/baseline/delta proof: authored, harness execution deferred.
#[path = "diagnostic_baseline_comparison/support.rs"]
mod fixture_helpers;
#[path = "diagnostic_baseline_comparison/reuse.rs"]
mod reuse;
use super::support;
use fixture_helpers::{invocation, run};
use saya_agent::CancellationToken;
use saya_harness::coding::commands::CommandOwner;
use saya_types::{CheckDiagnosticFormat, CodingCheckOutcome};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[tokio::test]
async fn existing_warning_does_not_hide_new_failure() {
    for format in [
        CheckDiagnosticFormat::RustcJson,
        CheckDiagnosticFormat::CargoJson,
    ] {
        let fixture = support::diagnostic_fixture("c04-real-delta").await;
        let owner = CommandOwner::new(1).unwrap();
        let invocation = invocation(format, false, Some("rust-stable"));
        let token = CancellationToken::new();
        let baseline_check = run(&fixture, &owner, &invocation).await;
        assert_eq!(
            baseline_check.receipt().outcome(),
            CodingCheckOutcome::KnownExit { code: 0 }
        );
        assert!(baseline_check.receipt().process().pid() > 0);
        assert_eq!(baseline_check.diagnostics().unwrap().entries().len(), 1);
        let ownership = fixture.ownership();
        let baseline = baseline_check.baseline(&ownership, &token).await.unwrap();
        let persisted = baseline_check.persisted_baseline(&baseline).unwrap();
        let persisted_bytes = persisted.to_bytes().unwrap();
        let persisted =
            saya_harness::coding::checks::PersistedDiagnosticBaseline::from_bytes(&persisted_bytes)
                .unwrap();
        fixture.change_candidate_source();
        let current = run(&fixture, &owner, &invocation).await;
        let delta = current
            .compare_persisted_baseline(&persisted, &ownership, &token)
            .await
            .unwrap();
        assert_eq!(delta.unchanged().len(), 1);
        assert_eq!(delta.unchanged()[0].code(), Some("unused_variables"));
        assert_eq!(delta.introduced().len(), 1);
        assert_eq!(delta.introduced()[0].code(), Some("E0425"));
        assert!(delta.resolved().is_empty());
        assert_ne!(delta.baseline_source(), delta.current_source());
        assert_eq!(
            delta.current_outcome(),
            CodingCheckOutcome::KnownExit { code: 1 }
        );
        assert!(!delta.current_successful());
        assert!(!current.receipt().is_successful());
        let mut tampered = persisted_bytes;
        tampered[0] = b'!';
        assert!(
            saya_harness::coding::checks::PersistedDiagnosticBaseline::from_bytes(&tampered)
                .is_err()
        );
    }
}

#[tokio::test]
async fn persisted_baseline_refuses_noncanonical_diagnostic_order() {
    let fixture = support::diagnostic_fixture("c04-baseline-order").await;
    fixture.change_candidate_source();
    let owner = CommandOwner::new(1).unwrap();
    let check = run(
        &fixture,
        &owner,
        &invocation(CheckDiagnosticFormat::RustcJson, false, None),
    )
    .await;
    let token = CancellationToken::new();
    let ownership = fixture.ownership();
    let baseline = check.baseline(&ownership, &token).await.unwrap();
    let persisted = check.persisted_baseline(&baseline).unwrap();
    let bytes = persisted.to_bytes().unwrap();
    let mut envelope: Value = serde_json::from_slice(&bytes).unwrap();
    envelope["payload"]["diagnostics"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let payload = serde_json::to_vec(&envelope["payload"]).unwrap();
    let digest = Sha256::digest(payload);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    envelope["payload_sha256"] = json!(hex);
    let tampered = serde_json::to_vec(&envelope).unwrap();
    assert!(matches!(
        saya_harness::coding::checks::PersistedDiagnosticBaseline::from_bytes(&tampered),
        Err(saya_harness::coding::checks::DiagnosticBaselineWireError::Malformed)
    ));
}

#[tokio::test]
async fn malformed_diagnostics_retain_real_nonzero_exit() {
    let fixture = support::diagnostic_fixture("c04-malformed").await;
    let owner = CommandOwner::new(1).unwrap();
    let invocation = invocation(CheckDiagnosticFormat::CargoJson, true, None);
    let check = run(&fixture, &owner, &invocation).await;
    assert_eq!(
        check.receipt().outcome(),
        CodingCheckOutcome::KnownExit { code: 23 }
    );
    assert!(!check.receipt().is_successful());
    assert!(check.diagnostics().is_err());
    let token = CancellationToken::new();
    let ownership = fixture.ownership();
    assert!(check.baseline(&ownership, &token).await.is_err());
    assert_eq!(check.receipt().process().exit_code(), Some(23));
}

#[tokio::test]
async fn known_nonzero_baseline_can_show_resolved_error_without_hiding_status() {
    let fixture = support::diagnostic_fixture("c04-resolved").await;
    fixture.change_candidate_source();
    let owner = CommandOwner::new(1).unwrap();
    let invocation = invocation(CheckDiagnosticFormat::RustcJson, false, None);
    let check = run(&fixture, &owner, &invocation).await;
    let token = CancellationToken::new();
    let ownership = fixture.ownership();
    let baseline = check.baseline(&ownership, &token).await.unwrap();
    fixture.set_candidate_source(b"baseline again\n");
    let current = run(&fixture, &owner, &invocation).await;
    let delta = current
        .compare_baseline(&baseline, &ownership, &token)
        .await
        .unwrap();
    assert_eq!(
        delta.baseline_outcome(),
        CodingCheckOutcome::KnownExit { code: 1 }
    );
    assert_eq!(delta.resolved().len(), 1);
    assert_eq!(delta.resolved()[0].code(), Some("E0425"));
    assert!(delta.introduced().is_empty());
    assert!(delta.current_successful());
}

#[tokio::test]
async fn duplicate_diagnostic_counts_are_preserved_in_both_directions() {
    let fixture = support::diagnostic_fixture("c04-multiset").await;
    let owner = CommandOwner::new(1).unwrap();
    let invocation = invocation(CheckDiagnosticFormat::RustcJson, false, None);
    let ownership = fixture.ownership();
    let token = CancellationToken::new();
    let one = run(&fixture, &owner, &invocation).await;
    let baseline = one.baseline(&ownership, &token).await.unwrap();
    fixture.set_candidate_source(b"duplicate warning\n");
    let two = run(&fixture, &owner, &invocation).await;
    let added = two
        .compare_baseline(&baseline, &ownership, &token)
        .await
        .unwrap();
    assert_eq!(added.unchanged().len(), 1);
    assert_eq!(added.introduced().len(), 1);
    assert!(added.resolved().is_empty());
    let baseline = two.baseline(&ownership, &token).await.unwrap();
    fixture.set_candidate_source(b"single warning again\n");
    let one = run(&fixture, &owner, &invocation).await;
    let removed = one
        .compare_baseline(&baseline, &ownership, &token)
        .await
        .unwrap();
    assert_eq!(removed.unchanged().len(), 1);
    assert_eq!(removed.resolved().len(), 1);
    assert!(removed.introduced().is_empty());
}

#[path = "diagnostic_baseline_comparison/provenance.rs"]
mod provenance;

#[tokio::test]
async fn empty_or_truncated_output_cannot_hide_nonzero_exit() {
    use saya_harness::coding::checks::{CheckDiagnosticError, DiagnosticEvidenceError};
    for (source, code) in [(b"empty".as_slice(), 19), (b"oversized".as_slice(), 31)] {
        let fixture = support::diagnostic_fixture("c04-output-completeness").await;
        fixture.set_candidate_source(source);
        let owner = CommandOwner::new(1).unwrap();
        let invocation = invocation(CheckDiagnosticFormat::RustcJson, false, None);
        let check = run(&fixture, &owner, &invocation).await;
        assert_eq!(
            check.receipt().outcome(),
            CodingCheckOutcome::KnownExit { code }
        );
        assert!(!check.receipt().is_successful());
        let ownership = fixture.ownership();
        let token = CancellationToken::new();
        if code == 19 {
            assert!(check.diagnostics().unwrap().entries().is_empty());
            check.baseline(&ownership, &token).await.unwrap();
        } else {
            assert!(matches!(
                check.diagnostics(),
                Err(CheckDiagnosticError::Capture(_))
            ));
            assert!(matches!(
                check.baseline(&ownership, &token).await,
                Err(DiagnosticEvidenceError::Incomplete)
            ));
        }
    }
}
