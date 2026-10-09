use super::*;

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
    e.source = EvidenceSource::Mcp;
    assert!(e.human_line().contains("mcp server"));
}
