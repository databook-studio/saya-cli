use super::*;

#[test]
fn round_trips_each_source_variant() {
    let sources = [
        EvidenceSource::DirectSql,
        EvidenceSource::SavedInvestigation {
            id: "inv-9".to_owned(),
            revision: 3,
        },
        EvidenceSource::Agent,
        EvidenceSource::Mcp,
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

#[test]
fn mcp_source_serializes_snake_case_tag() {
    let mut e = evidence();
    e.source = EvidenceSource::Mcp;
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"kind\":\"mcp\""), "{json}");
}
