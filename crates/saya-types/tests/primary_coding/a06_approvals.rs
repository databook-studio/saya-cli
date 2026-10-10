use std::num::NonZeroU64;

use saya_types::{
    AgentId, ApprovalNetworkDestination, ApprovalNetworkScheme, ApprovalRequestContext,
    ApprovalRequestError, ApprovalRequestV1, ApprovalScope, InvocationId, JobId, ManagedCheckoutId,
    ProfileIdentity, RepositoryId, RepositoryMutationAction, Sha256Digest,
};
use serde_json::{Value, json};

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PENDING: &str = r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"workspace_write","managed_checkout_id":"checkout-1","relative_path":"src/lib.rs"}}"#;

fn positive(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn request(scope: ApprovalScope) -> ApprovalRequestV1 {
    ApprovalRequestV1::new(
        JobId::parse("job-7").unwrap(),
        AgentId::parse("agent-2").unwrap(),
        InvocationId::parse("inv-4").unwrap(),
        positive(19),
        ApprovalRequestContext::new(Sha256Digest::parse(SHA_A).unwrap(), 0, 0, 5000).unwrap(),
        scope,
    )
    .unwrap()
}

fn parse(value: Value) -> Result<ApprovalRequestV1, ApprovalRequestError> {
    ApprovalRequestV1::from_json_bytes(&serde_json::to_vec(&value).unwrap())
}

#[test]
fn pending_request_preserves_exact_scope_policy_epoch_expiry_and_identity() {
    let parsed = ApprovalRequestV1::from_json_bytes(PENDING.as_bytes()).unwrap();
    let actual: Value = serde_json::from_slice(&parsed.to_json_bytes().unwrap()).unwrap();
    assert_eq!(actual, serde_json::from_str::<Value>(PENDING).unwrap());
}

#[test]
fn all_typed_scopes_round_trip_as_operation_specific_resources() {
    let records = [
        (
            ApprovalScope::workspace_write(
                ManagedCheckoutId::parse("checkout-1").unwrap(),
                "src/lib.rs",
            )
            .unwrap(),
            r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"workspace_write","managed_checkout_id":"checkout-1","relative_path":"src/lib.rs"}}"#,
        ),
        (
            ApprovalScope::process_execution(
                ManagedCheckoutId::parse("checkout-1").unwrap(),
                "cargo-test",
                Sha256Digest::parse(SHA_B).unwrap(),
                "tests",
            )
            .unwrap(),
            r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"process_execution","managed_checkout_id":"checkout-1","command_reference":"cargo-test","argv_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","cwd":"tests"}}"#,
        ),
        (
            ApprovalScope::network_access(
                ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, "db.example.test")
                    .unwrap(),
            ),
            r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"network_access","destination":{"scheme":"https","host":"db.example.test"}}}"#,
        ),
        (
            ApprovalScope::database_read(
                ProfileIdentity::parse(
                    "p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .unwrap(),
                Sha256Digest::parse(SHA_B).unwrap(),
            ),
            r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"database_read","profile_identity":"p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","statement_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}"#,
        ),
        (
            ApprovalScope::memory_write(RepositoryId::parse("repo-1").unwrap(), "coding.notes")
                .unwrap(),
            r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"memory_write","repository_id":"repo-1","memory_key":"coding.notes"}}"#,
        ),
        (
            ApprovalScope::repository_mutation(
                ManagedCheckoutId::parse("checkout-1").unwrap(),
                RepositoryMutationAction::ApplyBack,
            ),
            r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"repository_mutation","managed_checkout_id":"checkout-1","action":"apply_back"}}"#,
        ),
    ];
    for (scope, expected) in records {
        let record = request(scope);
        let wire = record.to_json_bytes().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&wire).unwrap(),
            serde_json::from_str::<Value>(expected).unwrap()
        );
        let decoded = ApprovalRequestV1::from_json_bytes(expected.as_bytes()).unwrap();
        assert_eq!(decoded.scope(), record.scope());
    }
}

#[test]
fn request_rejects_expiry_unknown_versions_grants_and_invalid_resources() {
    for expires in [0, 1] {
        let mut value: Value = serde_json::from_str(PENDING).unwrap();
        value["requested_at_unix_ms"] = json!(1);
        value["expires_at_unix_ms"] = json!(expires);
        assert!(parse(value).is_err());
        assert_eq!(
            ApprovalRequestContext::new(Sha256Digest::parse(SHA_A).unwrap(), 0, 1, expires),
            Err(ApprovalRequestError::InvalidExpiry)
        );
    }
    let mut future: Value = serde_json::from_str(PENDING).unwrap();
    future["version"] = json!(2);
    assert_eq!(
        parse(future),
        Err(ApprovalRequestError::UnsupportedVersion(2))
    );

    let mut grant: Value = serde_json::from_str(PENDING).unwrap();
    grant["session_grant"] = json!(true);
    assert_eq!(parse(grant), Err(ApprovalRequestError::Malformed));

    let mut scope_grant: Value = serde_json::from_str(PENDING).unwrap();
    scope_grant["scope"]["grant"] = json!(true);
    assert_eq!(parse(scope_grant), Err(ApprovalRequestError::Malformed));

    let mut destination_grant: Value = serde_json::from_str(PENDING).unwrap();
    destination_grant["scope"] = json!({"operation":"network_access","destination":{"scheme":"https","host":"db.example.test","grant":true}});
    assert_eq!(
        parse(destination_grant),
        Err(ApprovalRequestError::Malformed)
    );

    let mut zero_sequence: Value = serde_json::from_str(PENDING).unwrap();
    zero_sequence["request_sequence"] = json!(0);
    assert_eq!(parse(zero_sequence), Err(ApprovalRequestError::Malformed));

    for path in [
        "../secret",
        "/absolute",
        "a\\b",
        "a//b",
        "a/./b",
        ".",
        "C:relative",
        "bad\npath",
    ] {
        let mut invalid: Value = serde_json::from_str(PENDING).unwrap();
        invalid["scope"]["relative_path"] = json!(path);
        assert_eq!(parse(invalid), Err(ApprovalRequestError::Malformed));
        assert!(
            ApprovalScope::workspace_write(ManagedCheckoutId::parse("checkout-1").unwrap(), path,)
                .is_err()
        );
    }
    assert!(
        ApprovalScope::process_execution(
            ManagedCheckoutId::parse("checkout-1").unwrap(),
            "../run",
            Sha256Digest::parse(SHA_B).unwrap(),
            ".",
        )
        .is_err()
    );
    assert!(
        ApprovalScope::memory_write(RepositoryId::parse("repo-1").unwrap(), "bad/key").is_err()
    );
    let mut invalid_memory_key: Value = serde_json::from_str(PENDING).unwrap();
    invalid_memory_key["scope"] =
        json!({"operation":"memory_write","repository_id":"repo-1","memory_key":"bad/key"});
    assert_eq!(
        parse(invalid_memory_key),
        Err(ApprovalRequestError::Malformed)
    );

    for host in [
        "user@db.example",
        "db.example:443",
        "db.example/path",
        "db.example?q=1",
        "-bad.example",
        "db..example",
        "db.example.",
    ] {
        assert!(ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, host).is_err());
    }
    let mut unsafe_destination: Value = serde_json::from_str(PENDING).unwrap();
    unsafe_destination["scope"] = json!({"operation":"network_access","destination":{"scheme":"https","host":"user@db.example"}});
    assert_eq!(
        parse(unsafe_destination),
        Err(ApprovalRequestError::Malformed)
    );
    assert_eq!(
        ApprovalNetworkDestination::new(ApprovalNetworkScheme::Http, "DB.Example.Test")
            .unwrap()
            .host(),
        "db.example.test"
    );
    let mut unsupported_scheme: Value = serde_json::from_str(PENDING).unwrap();
    unsupported_scheme["scope"] = json!({"operation":"network_access","destination":{"scheme":"ftp","host":"db.example.test"}});
    assert_eq!(
        parse(unsupported_scheme),
        Err(ApprovalRequestError::Malformed)
    );
    for host in [
        "127.1",
        "2130706433",
        "0177.0.0.1",
        "0x7f000001",
        "0x7f.0.0.1",
        "0x",
        "0X7F000001",
    ] {
        let mut ambiguous_host: Value = serde_json::from_str(PENDING).unwrap();
        ambiguous_host["scope"] =
            json!({"operation":"network_access","destination":{"scheme":"https","host":host}});
        assert_eq!(parse(ambiguous_host), Err(ApprovalRequestError::Malformed));
    }
    assert!(
        ApprovalScope::process_execution(
            ManagedCheckoutId::parse("checkout-1").unwrap(),
            "cargo-test",
            Sha256Digest::parse(SHA_B).unwrap(),
            ".",
        )
        .is_ok()
    );
    let mut unsafe_command = serde_json::from_str::<Value>(PENDING).unwrap();
    unsafe_command["scope"] = json!({"operation":"process_execution","managed_checkout_id":"checkout-1","command_reference":"../run","argv_sha256":SHA_B,"cwd":"."});
    assert_eq!(parse(unsafe_command), Err(ApprovalRequestError::Malformed));

    let mut escaping_cwd: Value = serde_json::from_str(PENDING).unwrap();
    escaping_cwd["scope"] = json!({"operation":"process_execution","managed_checkout_id":"checkout-1","command_reference":"cargo-test","argv_sha256":SHA_B,"cwd":"../escape"});
    assert_eq!(parse(escaping_cwd), Err(ApprovalRequestError::Malformed));

    let mut root_cwd: Value = serde_json::from_str(PENDING).unwrap();
    root_cwd["scope"] = json!({"operation":"process_execution","managed_checkout_id":"checkout-1","command_reference":"cargo-test","argv_sha256":SHA_B,"cwd":"."});
    assert!(parse(root_cwd).is_ok());

    let mut mismatched_operation: Value = serde_json::from_str(PENDING).unwrap();
    mismatched_operation["scope"]["operation"] = json!("process_execution");
    assert_eq!(
        parse(mismatched_operation),
        Err(ApprovalRequestError::Malformed)
    );

    for (action, wire) in [
        (RepositoryMutationAction::CreateCheckout, "create_checkout"),
        (RepositoryMutationAction::RemoveCheckout, "remove_checkout"),
        (
            RepositoryMutationAction::IntegrateCandidate,
            "integrate_candidate",
        ),
        (RepositoryMutationAction::ApplyBack, "apply_back"),
    ] {
        let record = request(ApprovalScope::repository_mutation(
            ManagedCheckoutId::parse("checkout-1").unwrap(),
            action,
        ));
        let value: Value = serde_json::from_slice(&record.to_json_bytes().unwrap()).unwrap();
        assert_eq!(value["scope"]["action"], json!(wire));
    }
}

#[test]
fn request_wire_limit_accepts_exact_limit_and_rejects_one_byte_over() {
    let fixture = PENDING.to_string();
    let exact = format!("{fixture}{}", " ".repeat(65_536 - fixture.len()));
    assert_eq!(exact.len(), 65_536);
    assert!(ApprovalRequestV1::from_json_bytes(exact.as_bytes()).is_ok());
    let oversized = format!("{exact} ");
    assert_eq!(
        ApprovalRequestV1::from_json_bytes(oversized.as_bytes()),
        Err(ApprovalRequestError::Oversize(65_537))
    );
}
