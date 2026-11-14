use super::*;

#[test]
fn for_result_copies_counts_truncation_and_hashes_submitted_sql() {
    let e = evidence();
    assert_eq!(e.returned_rows, 12);
    assert_eq!(e.max_rows, 1000);
    assert!(!e.truncated);
    assert_eq!(e.submitted_sql_sha256, SELECT_ONE_SHA256);
    assert_eq!(e.submitted_sql_sha256.len(), 64);
    assert!(
        e.submitted_sql_sha256
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
    assert_eq!(e.scope, ResultScope::Full);
    assert_eq!(e.dialect, SqlDialect::Postgres);
    assert_eq!(e.connection_label, "warehouse");
    assert_eq!(
        e.connection_identity.as_deref(),
        Some("postgres://ops@db.internal:5432/prod")
    );
    assert!(e.knowledge_ids.is_empty());
    assert_eq!(e.started_unix_ms, 1_790_000_000_000);
    assert_eq!(e.finished_unix_ms, 1_790_000_000_900);
}

#[test]
fn for_result_copies_truncation_flag() {
    let mut result = simple_result("SELECT 1");
    result.truncated = true;
    let e = ExecutionEvidence::for_result(&result, base_args());
    assert!(e.truncated);
}

#[test]
fn hash_differs_when_submitted_sql_differs() {
    let a = ExecutionEvidence::for_result(&simple_result("SELECT 1"), base_args());
    let b = ExecutionEvidence::for_result(&simple_result("SELECT 2"), base_args());
    assert_ne!(a.submitted_sql_sha256, b.submitted_sql_sha256);
}

#[test]
fn serialized_json_omits_sql_text() {
    let e = ExecutionEvidence::for_result(&simple_result("SELECT 'secret-sentinel'"), base_args());
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("submitted_sql_sha256"));
    assert!(!json.contains("secret-sentinel"));
    assert!(!json.contains("SELECT"));
}
