//! Tests for the execution-evidence contract (`saya-types::evidence`).

use super::*;
use crate::dialect::SqlDialect;
use crate::query::QueryResult;

const SELECT_ONE_SHA256: &str = "e004ebd5b5532a4b85984a62f8ad48a81aa3460c1ca07701f386135d72cdecf5";

fn simple_result(sql: &str) -> QueryResult {
    QueryResult {
        columns: vec!["?column?".to_owned()],
        rows: vec![serde_json::json!(1)],
        row_count: 12,
        truncated: false,
        executed_sql: sql.to_owned(),
    }
}

fn base_args() -> ExecutionEvidenceArgs {
    ExecutionEvidenceArgs {
        execution_id: "x1a2b3c-4".to_owned(),
        connection_label: "warehouse".to_owned(),
        connection_identity: Some("postgres://ops@db.internal:5432/prod".to_owned()),
        dialect: SqlDialect::Postgres,
        max_rows: 1000,
        started_unix_ms: 1_790_000_000_000,
        finished_unix_ms: 1_790_000_000_900,
        source: EvidenceSource::DirectSql,
    }
}

fn evidence() -> ExecutionEvidence {
    ExecutionEvidence::for_result(&simple_result("SELECT 1"), base_args())
}

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
fn round_trips_each_source_variant() {
    let sources = [
        EvidenceSource::DirectSql,
        EvidenceSource::SavedInvestigation {
            id: "inv-9".to_owned(),
            revision: 3,
        },
        EvidenceSource::Agent,
    ];
    for source in sources {
        let mut e = evidence();
        e.source = source.clone();
        let json = serde_json::to_string(&e).unwrap();
        let back: ExecutionEvidence = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.source, source);
    }
}

#[test]
fn saved_investigation_serializes_snake_case_tag() {
    let mut e = evidence();
    e.source = EvidenceSource::SavedInvestigation {
        id: "inv-9".to_owned(),
        revision: 3,
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"kind\":\"saved_investigation\""));
    assert!(json.contains("\"id\":\"inv-9\""));
    assert!(json.contains("\"revision\":3"));
}

#[test]
fn round_trips_each_scope_variant() {
    let scopes = [ResultScope::Full, ResultScope::ModelLimited { row_cap: 50 }];
    for scope in scopes {
        let mut e = evidence();
        e.scope = scope.clone();
        let json = serde_json::to_string(&e).unwrap();
        let back: ExecutionEvidence = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.scope, scope);
    }
}

#[test]
fn knowledge_ids_default_to_empty_when_absent() {
    let json = serde_json::json!({
        "execution_id": "x1",
        "submitted_sql_sha256": "a".repeat(64),
        "connection_label": "warehouse",
        "connection_identity": null,
        "dialect": "postgresql",
        "schema_fingerprint": null,
        "started_unix_ms": 1_790_000_000_000_i64,
        "finished_unix_ms": 1_790_000_000_900_i64,
        "max_rows": 1000,
        "returned_rows": 12,
        "truncated": false,
        "scope": { "kind": "full" },
        "source": { "kind": "direct_sql" },
    })
    .to_string();
    let e: ExecutionEvidence = serde_json::from_str(&json).unwrap();
    assert!(e.knowledge_ids.is_empty());
    assert_eq!(e.dialect, SqlDialect::Postgres);
}

#[test]
fn knowledge_ids_are_clamped_to_the_bound() {
    let ids: Vec<String> = (0..MAX_EVIDENCE_KNOWLEDGE_IDS + 10)
        .map(|i| format!("kb-{i}"))
        .collect();
    let e = evidence().with_knowledge_ids(ids);
    assert_eq!(e.knowledge_ids.len(), MAX_EVIDENCE_KNOWLEDGE_IDS);
}

/// The parameter fields default to absent for a parameter-free run, and a
/// bound run carries names and a digest — never a value (B1f).
#[test]
fn parameter_fields_default_absent_and_carry_names_with_a_digest_only() {
    let bare = evidence();
    assert!(bare.param_names.is_empty());
    assert_eq!(bare.params_sha256, None);
    let json = serde_json::to_string(&bare).unwrap();
    assert!(
        !json.contains("param_names") && !json.contains("params_sha256"),
        "empty parameter fields are omitted from serialization: {json}"
    );

    let bound = evidence().with_param_bindings(
        vec!["label".to_owned(), "since".to_owned()],
        Some("a".repeat(64)),
    );
    assert_eq!(
        bound.param_names,
        vec!["label".to_owned(), "since".to_owned()]
    );
    assert_eq!(bound.params_sha256.as_deref(), Some(&"a".repeat(64)[..]));
    let json = serde_json::to_string(&bound).unwrap();
    assert!(
        json.contains("\"param_names\":[\"label\",\"since\"]"),
        "{json}"
    );
    assert!(json.contains("\"params_sha256\""), "{json}");
    assert!(
        !json.contains("value"),
        "the serialized evidence carries names and a digest, never a value: {json}"
    );

    // The digest round-trips: an evidence record deserializes with both
    // fields intact.
    let back: ExecutionEvidence =
        serde_json::from_str(&serde_json::to_string(&bound).unwrap()).unwrap();
    assert_eq!(back, bound);
}

/// The evidence line records the parameters' names — never a value.
#[test]
fn human_line_records_parameter_names_only() {
    let line = evidence()
        .with_param_bindings(vec!["label".to_owned()], Some("b".repeat(64)))
        .human_line();
    assert!(line.contains("params: label"), "{line}");
    assert!(!line.contains("params_sha256"), "{line}");
    let bare = evidence().human_line();
    assert!(!bare.contains("params:"), "{bare}");
}

#[test]
fn human_line_shows_label_and_never_identity() {
    let line = evidence().human_line();
    assert!(line.contains("warehouse"));
    assert!(!line.contains("db.internal"));
    assert!(!line.contains("postgres://"));
    assert!(line.contains("12 rows"));
    assert!(line.contains("exec x1a2b3c-4"));
    assert!(line.contains("full result"));
    assert!(!line.contains("SELECT"));
}

#[test]
fn human_line_mentions_truncation_only_when_truncated() {
    let clean = evidence().human_line();
    assert!(!clean.contains("truncated"));

    let mut result = simple_result("SELECT 1");
    result.row_count = 1000;
    result.truncated = true;
    let line = ExecutionEvidence::for_result(&result, base_args()).human_line();
    assert!(line.contains("1000 rows (truncated at 1000)"));
}

#[test]
fn human_line_describes_model_limited_scope() {
    let mut e = evidence();
    e.scope = ResultScope::ModelLimited { row_cap: 50 };
    let line = e.human_line();
    assert!(line.contains("model-limited (first 50 rows)"));
    assert!(!line.contains("full result"));
}

#[test]
fn human_line_describes_each_source() {
    let mut e = evidence();
    e.source = EvidenceSource::DirectSql;
    assert!(e.human_line().contains("direct sql"));
    e.source = EvidenceSource::SavedInvestigation {
        id: "inv-9".to_owned(),
        revision: 3,
    };
    assert!(e.human_line().contains("saved investigation"));
    e.source = EvidenceSource::Agent;
    assert!(e.human_line().contains("agent"));
}

#[test]
fn serialized_json_omits_sql_text() {
    let e = ExecutionEvidence::for_result(&simple_result("SELECT 'secret-sentinel'"), base_args());
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("submitted_sql_sha256"));
    assert!(!json.contains("secret-sentinel"));
    assert!(!json.contains("SELECT"));
}

#[test]
fn new_execution_id_is_deterministic_and_charset_bounded() {
    let a = ExecutionEvidence::new_execution_id(1_790_000_000_000, 7);
    let b = ExecutionEvidence::new_execution_id(1_790_000_000_000, 7);
    assert_eq!(a, b);
    assert!(a.starts_with('x'));
    assert!(a.contains('-'));
    assert!(a.len() <= 32);
    let on_charset = |id: &str| {
        id.bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    assert!(on_charset(&a));
    assert_ne!(
        ExecutionEvidence::new_execution_id(1_790_000_000_000, 7),
        ExecutionEvidence::new_execution_id(1_790_000_000_000, 8)
    );
    assert_ne!(
        ExecutionEvidence::new_execution_id(1_790_000_000_000, 7),
        ExecutionEvidence::new_execution_id(1_790_000_001_000, 7)
    );
    let extremes = ExecutionEvidence::new_execution_id(i64::MIN, u64::MAX);
    assert!(extremes.len() <= 32);
    assert!(on_charset(&extremes));
}

#[test]
fn short_id_caps_at_twelve_characters() {
    let mut e = evidence();
    e.execution_id = "xabcdefghijkmnop".to_owned();
    assert_eq!(e.short_id(), "xabcdefghijk");
    e.execution_id = "x1a2b3c-4".to_owned();
    assert_eq!(e.short_id(), "x1a2b3c-4");
}
