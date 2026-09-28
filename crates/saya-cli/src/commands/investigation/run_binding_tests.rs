//! The staleness matrix for `run_binding::staleness` (invariant 3): a
//! binding goes stale when the definition revision, the target profile, the
//! target identity, or the recorded schema fingerprint no longer matches;
//! an absent binding is never stale.

use super::{StaleReason, stale_message, staleness};
use saya_store::LocalBinding;
use saya_types::SqlDialect;
use saya_types::investigation::{
    INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1, InvestigationId,
};

const ID: &str = "order-events-ab12cd34";
const IDENTITY: &str = "p-0000000000000000000000000000000000000000000000000000000000000000";

fn definition(revision: u32, objects: Vec<String>) -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_string(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: InvestigationId::parse(ID).unwrap(),
        revision,
        name: "Order events".into(),
        description: None,
        sql: "SELECT id, label FROM events".into(),
        dialect: SqlDialect::Sqlite,
        connection: "local".into(),
        objects,
        schema_fingerprint: None,
        created_unix_ms: 1_000,
        updated_unix_ms: 1_000,
    }
}

fn binding(
    revision: u32,
    profile: &str,
    identity: &str,
    fingerprint: Option<&str>,
) -> LocalBinding {
    LocalBinding {
        version: LocalBinding::VERSION,
        id: InvestigationId::parse(ID).unwrap(),
        profile: profile.to_string(),
        profile_identity: identity.to_string(),
        reviewed_revision: revision,
        reviewed_schema_fingerprint: fingerprint.map(str::to_owned),
        reviewed_unix_ms: 1_000,
    }
}

#[test]
fn no_binding_is_never_stale() {
    let decision = staleness(
        None,
        &definition(1, vec!["events".into()]),
        "local",
        IDENTITY,
        Some("f1"),
    );
    assert_eq!(decision, Vec::new());
}

#[test]
fn matching_binding_is_not_stale() {
    let binding = binding(1, "local", IDENTITY, Some("f1"));
    let decision = staleness(
        Some(&binding),
        &definition(1, vec!["events".into()]),
        "local",
        IDENTITY,
        Some("f1"),
    );
    assert_eq!(decision, Vec::new());
}

#[test]
fn revision_changed_is_stale() {
    let binding = binding(1, "local", IDENTITY, Some("f1"));
    let decision = staleness(
        Some(&binding),
        &definition(2, vec!["events".into()]),
        "local",
        IDENTITY,
        Some("f1"),
    );
    assert_eq!(decision, vec![StaleReason::Revision]);
}

#[test]
fn profile_name_changed_is_target_stale() {
    let binding = binding(1, "local", IDENTITY, Some("f1"));
    let decision = staleness(
        Some(&binding),
        &definition(1, vec!["events".into()]),
        "other",
        IDENTITY,
        Some("f1"),
    );
    assert_eq!(decision, vec![StaleReason::Target]);
}

#[test]
fn identity_changed_is_target_stale() {
    let binding = binding(1, "local", IDENTITY, Some("f1"));
    let decision = staleness(
        Some(&binding),
        &definition(1, vec!["events".into()]),
        "local",
        "p-ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        Some("f1"),
    );
    assert_eq!(decision, vec![StaleReason::Target]);
}

#[test]
fn fingerprint_changed_is_schema_stale() {
    let binding = binding(1, "local", IDENTITY, Some("f1"));
    let decision = staleness(
        Some(&binding),
        &definition(1, vec!["events".into()]),
        "local",
        IDENTITY,
        Some("f2"),
    );
    assert_eq!(decision, vec![StaleReason::Schema]);
}

#[test]
fn binding_fingerprint_some_current_none_is_schema_stale() {
    let binding = binding(1, "local", IDENTITY, Some("f1"));
    let decision = staleness(
        Some(&binding),
        &definition(1, vec![]),
        "local",
        IDENTITY,
        None,
    );
    assert_eq!(decision, vec![StaleReason::Schema]);
}

#[test]
fn binding_fingerprint_none_is_never_schema_stale() {
    let binding = binding(1, "local", IDENTITY, None);
    let decision = staleness(
        Some(&binding),
        &definition(1, vec!["events".into()]),
        "local",
        IDENTITY,
        Some("f1"),
    );
    assert_eq!(decision, Vec::new());
}

#[test]
fn every_reason_is_named_at_once_in_order() {
    let binding = binding(1, "local", IDENTITY, Some("f1"));
    let decision = staleness(
        Some(&binding),
        &definition(2, vec!["events".into()]),
        "other",
        "p-ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        Some("f2"),
    );
    assert_eq!(
        decision,
        vec![
            StaleReason::Revision,
            StaleReason::Target,
            StaleReason::Schema
        ]
    );
    let message = stale_message(&decision);
    assert!(message.contains("revision changed"), "{message}");
    assert!(message.contains("target changed"), "{message}");
    assert!(message.contains("schema changed"), "{message}");
    assert!(message.contains("--revalidate"), "{message}");
}
