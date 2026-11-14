//! The bounded paginated load behind the picker: more than one page of 50
//! is pulled in id order, and the picker cap bounds it.

use super::super::load_investigation_entries;
use super::definition;
use saya_store::InvestigationRepository;
use std::time::{SystemTime, UNIX_EPOCH};

/// More than one page of 50 is pulled through pagination, in id order —
/// the load never stops at the first page, and the picker cap (500, the
/// collection cap the store itself enforces on create) bounds it.
#[test]
fn load_pulls_more_than_one_page_in_id_order() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-picker-pages-{}-{nanos}",
        std::process::id()
    ));
    let repo = InvestigationRepository::new(root.clone());
    for i in 0..60 {
        repo.create(&definition(&format!("doc{i:03}-aaaaaaaa"), 1))
            .unwrap();
    }
    let (entries, capped) = load_investigation_entries(&root).unwrap();
    assert_eq!(
        entries.len(),
        60,
        "pagination continues past the first page of 50"
    );
    assert!(!capped, "the collection cap was not reached");
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "entries stay in id order");
    let _ = std::fs::remove_dir_all(&root);
}
