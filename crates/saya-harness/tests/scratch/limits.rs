use super::*;

#[test]
fn sql_over_the_preparse_bound_is_refused_before_sqlparser() {
    let sql = "x".repeat(MAX_SQL_BYTES + 1);
    let error = validate(&sql).expect_err("oversized SQL must be refused");
    assert!(matches!(error, ScratchRejection::TooLarge { .. }));
}

/// A019: the pre-parser admission boundary is exact. A payload of exactly
/// `MAX_SQL_BYTES` passes the byte gate (it may still fail parsing on its own
/// merits), while one byte more is refused as `TooLarge` without parsing. Both
/// go through `validate` — the admission boundary `ScratchSql::run` enters —
/// and both payloads are adversarial-shaped (unparsable `x` runs), so a pass
/// proves the byte gate admitted the input rather than the parser being cheap.
#[tokio::test]
async fn scratch_sql_admits_exactly_at_the_preparse_byte_bound() {
    let run = TempRun::new("exact-limit");
    let tool = admitted(&run);
    // Exactly at the limit: the byte gate admits; the payload itself is
    // unparsable, so the failure must come from the parser, not the gate.
    let sql = "x".repeat(MAX_SQL_BYTES);
    assert_eq!(sql.len(), MAX_SQL_BYTES);
    match tool.run(&sql).await {
        Err(ScratchError::Refused(ScratchRejection::Unparsable)) => {}
        other => panic!("at-limit input must reach the parser, got {other:?}"),
    }
    // One byte over: the byte gate refuses before the parser runs.
    let sql = "x".repeat(MAX_SQL_BYTES + 1);
    match tool.run(&sql).await {
        Err(ScratchError::Refused(ScratchRejection::TooLarge { bytes, max })) => {
            assert_eq!(bytes, MAX_SQL_BYTES + 1);
            assert_eq!(max, MAX_SQL_BYTES);
        }
        other => panic!("over-limit input must be refused pre-parse, got {other:?}"),
    }
    drop(tool);
    let _ = fs::remove_dir_all(&run.runs_root);
}
