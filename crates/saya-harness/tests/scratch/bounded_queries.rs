use super::*;

#[tokio::test]
async fn create_insert_select_round_trip_works_and_reads_stay_capped() {
    let run = TempRun::new("round-trip");
    let tool = admitted(&run);

    let created = tool
        .run("CREATE TABLE predictions(id INTEGER, gold INTEGER, pred INTEGER)")
        .await
        .unwrap();
    assert_eq!(created.row_count, 0);

    let inserted = tool
        .run("INSERT INTO predictions VALUES (1, 10, 9), (2, 20, 20)")
        .await
        .unwrap();
    assert_eq!(inserted.row_count, 2);

    let selected = tool
        .run("SELECT id, gold, pred FROM predictions ORDER BY id")
        .await
        .unwrap();
    assert_eq!(selected.columns, ["id", "gold", "pred"]);
    assert_eq!(selected.rows, json_rows(json!([[1, 10, 9], [2, 20, 20]])));

    let joined = tool
        .run("SELECT count(*) AS misses FROM predictions WHERE gold != pred")
        .await
        .unwrap();
    assert_eq!(joined.rows, json_rows(json!([[1]])));

    // The 50-row discipline: 60 staged rows come back as exactly 50 with
    // `truncated` set, and the executed SQL is the statement the caller gave.
    let staged = tool
        .run("INSERT INTO predictions SELECT range, range, range FROM range(60)")
        .await
        .unwrap();
    assert_eq!(staged.row_count, 60);
    let sql = "SELECT id FROM predictions ORDER BY id";
    let capped = tool.run(sql).await.unwrap();
    assert_eq!(capped.rows.len(), 50);
    assert!(capped.truncated);
    assert_eq!(capped.executed_sql, sql);

    // An explicit bound at or under the cap is left as written.
    let bounded = tool
        .run("SELECT id FROM predictions LIMIT 5")
        .await
        .unwrap();
    assert_eq!(bounded.rows.len(), 5);
    assert!(!bounded.truncated);

    // UPDATE and DELETE stay available on the staged rows.
    let updated = tool
        .run("UPDATE predictions SET pred = gold WHERE id > 55")
        .await
        .unwrap();
    assert!(updated.row_count > 0);
    let deleted = tool
        .run("DELETE FROM predictions WHERE id > 40")
        .await
        .unwrap();
    assert!(deleted.row_count > 0);

    // SHOW still reads.
    let shown = tool.run("SHOW TABLES").await.unwrap();
    assert_eq!(shown.row_count, 1);
}

/// No type-level path may run from a scratch write to a user database. The
/// proof is structural, in two halves:
///
/// 1. The harness crate cannot even *name* the connector crate: its shipped
///    `[dependencies]` never list `saya-connectors`, so no type in this crate
///    can implement `DatabaseConnector`, and the `ConnectionRegistry` — which
///    stores `Box<dyn DatabaseConnector>` — cannot hold one.
/// 2. The scratch module's *code* never names the types it would need to fake
///    it, guarding against a future dependency being slipped in quietly. The
///    scan strips comments so honest docs about the separation are fine; only
///    code can trip it.
#[test]
fn scratch_has_no_type_level_path_to_user_databases() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let shipped = manifest_section(&manifest, "[dependencies]");
    assert!(
        !shipped.contains("saya-connectors"),
        "saya-harness must not depend on the connector crate: scratch has no \
         connector path by construction"
    );

    let scratch_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/scratch");
    for entry in fs::read_dir(&scratch_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let code = code_only(&fs::read_to_string(&path).unwrap());
        for forbidden in [
            "DatabaseConnector",
            "ConnectionRegistry",
            "ConnectionEntry",
            "saya_connectors",
        ] {
            assert!(
                !code.contains(forbidden),
                "{} names {forbidden}: the scratch surface must never reference \
                 the user-database connector path",
                path.display()
            );
        }
    }
}
