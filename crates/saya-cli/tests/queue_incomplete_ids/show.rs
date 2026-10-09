//! The show arm of the A013 regression: a malformed selected id applies the
//! same visible-incompleteness rule to the selected rows.

use super::*;

#[tokio::test]
async fn show_marks_a_malformed_selected_id_incomplete_and_keeps_the_valid_claim() {
    let root = temp_root("malformed-selected");
    let runtime = runtime_at(&root);
    let db = root.join("state.sqlite3");
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();

    let object = object_for(&runtime, "orders");
    seed_pending_alias(&store, &object, "customers").await;
    plant_malformed_pending_id(&store, &db, &object).await;

    let show = ContractsCommand::Show {
        table: "analytics.public.orders".into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("customers"),
        "the valid claim must still render: {out}"
    );
    assert!(
        !out.contains("not a valid id!"),
        "the malformed id must never render: {out}"
    );
    assert!(
        out.contains("incomplete"),
        "show must visibly disclose the dropped malformed row: {out}"
    );

    let _ = fs::remove_dir_all(root);
}
