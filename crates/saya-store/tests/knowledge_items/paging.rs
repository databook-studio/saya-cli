use super::*;

#[tokio::test]
async fn paged_profile_reads_are_bounded_deterministic_and_continuable() {
    let root = temp_root("pages");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let prof = profile('a');
    for name in ["zulu", "alpha", "mango"] {
        let obj = object(&prof, name);
        store
            .put_knowledge_item(request(
                &obj,
                KnowledgeSlot::TableGrain,
                ClaimPayload::table_grain(name, None).unwrap(),
                ClaimOrigin::UserExplicit,
                KnowledgeState::Active,
                1,
            ))
            .await
            .unwrap();
    }

    assert!(KnowledgeItemsQuery::first_page(0).is_err());
    assert!(KnowledgeItemsQuery::first_page(MAX_KNOWLEDGE_PAGE_SIZE + 1).is_err());
    assert!(KnowledgeObjectsQuery::first_page(0).is_err());

    let mut query = KnowledgeItemsQuery::first_page(1).unwrap();
    let mut names = Vec::new();
    let mut pages = 0;
    loop {
        let page = store
            .knowledge_for_profile_page(&prof, query.clone())
            .await
            .unwrap();
        pages += 1;
        assert!(page.entries.len() <= 1);
        names.extend(
            page.entries
                .iter()
                .map(|item| item.object.object().to_owned()),
        );
        let Some(next) = query.next_page(&page) else {
            break;
        };
        query = next;
    }
    assert_eq!(pages, 3);
    assert_eq!(names, ["alpha", "mango", "zulu"]);
    let _ = fs::remove_dir_all(root);
}
