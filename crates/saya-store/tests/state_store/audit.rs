use super::*;

#[tokio::test]
async fn audit_is_typed_bounded_retained_and_decoded_without_payload_fields() {
    let root = temp_root("audit");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    for index in 0..1_005 {
        let mut event = AuditEntry::new(PROFILE, AuditOperation::Query, AuditStatus::Success, 5);
        event.row_count = Some(index);
        event.truncated = Some(false);
        store.record_audit(event).await.unwrap();
    }
    let audit = store.recent_audit(2_000).await.unwrap();
    assert_eq!(audit.len(), 1_000);
    assert!(
        audit
            .iter()
            .all(|row| row.event.operation == AuditOperation::Query
                && row.event.profile_id == PROFILE)
    );
    assert!(store.recent_audit(20_000).await.unwrap().len() <= 1_000);
    assert!(store.recent_audit(0).await.unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}
