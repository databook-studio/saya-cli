/// Two store roots handed out by the helper must own different lifecycle
/// parents: tombstones live in `<store root>/../lifecycle`, so roots that
/// share a parent share every tombstone — a concurrent delete test's
/// tombstone leaked into another test's "empty store" listing. A tombstone
/// begun through one root stays invisible to the other.
#[test]
fn two_store_roots_from_the_helper_keep_their_tombstones_apart() {
    let root_a = scratch("isolate-a");
    let root_b = scratch("isolate-b");
    let parent_a = root_a.parent().expect("scratch parent").to_path_buf();
    let parent_b = root_b.parent().expect("scratch parent").to_path_buf();
    assert_ne!(
        parent_a, parent_b,
        "two scratch store roots share a lifecycle parent: {parent_a:?}"
    );
    assert_ne!(
        parent_a.join("lifecycle"),
        parent_b.join("lifecycle"),
        "the tombstone directories must be distinct paths"
    );
    // A tombstone-only delete begun through root A (no record, no live
    // lock: the tombstone survives) is root A's pending deletion alone.
    let store_a = FsSessionStore::new(&root_a);
    store_a.begin_delete("a").expect("the tombstone is written");
    assert_eq!(
        block_on(list_sessions(&store_a)).unwrap().pending,
        vec!["a".to_owned()],
        "root A sees its own tombstone"
    );
    let store_b = FsSessionStore::new(&root_b);
    assert!(
        block_on(list_sessions(&store_b))
            .unwrap()
            .pending
            .is_empty(),
        "root B must not see root A's tombstone"
    );
    let _ = std::fs::remove_dir_all(parent_a);
    let _ = std::fs::remove_dir_all(parent_b);
}
