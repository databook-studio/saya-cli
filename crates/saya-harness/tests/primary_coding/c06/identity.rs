use saya_harness::coding::skills::{SkillLoadError, SkillOrigin};

use super::support::{TempTree, entry_for, load, root};

#[test]
fn rewritten_frontmatter_between_discovery_and_load_refuses_as_metadata_changed() {
    let tree = TempTree::new();
    tree.skill("sample", "sample", "first description", b"body\n");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "sample");

    tree.write_skill_file(
        "sample",
        &tree.skill_bytes("sample", "second description", b"body\n"),
    );
    assert!(matches!(
        load(&root, &entry),
        Err(SkillLoadError::MetadataChanged)
    ));

    tree.write_skill_file(
        "sample",
        &tree.skill_bytes("renamed", "first description", b"body\n"),
    );
    assert!(matches!(
        load(&root, &entry),
        Err(SkillLoadError::MetadataChanged)
    ));
}

#[test]
fn tampered_entry_name_with_intact_file_and_digest_refuses_as_metadata_changed() {
    let tree = TempTree::new();
    tree.skill("sample", "sample", "described", b"body\n");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "sample");

    // Only the discovered entry's name is tampered with: the file and its
    // recorded metadata digest stay intact, so the name comparison itself
    // must carry the refusal.
    let mut tampered = entry.clone();
    tampered.name = "tampered".to_string();
    load(&root, &entry).expect("the intact entry should still load");
    assert!(matches!(
        load(&root, &tampered),
        Err(SkillLoadError::MetadataChanged)
    ));
}

#[test]
fn malformed_reopened_frontmatter_refuses_as_metadata_changed() {
    let tree = TempTree::new();
    tree.skill("sample", "sample", "described", b"body\n");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "sample");

    tree.write_skill_file("sample", b"---\nname: sample\ndescription: described\n");
    assert!(matches!(
        load(&root, &entry),
        Err(SkillLoadError::MetadataChanged)
    ));
}

#[test]
fn body_only_edits_still_load_but_change_the_snapshot_digest() {
    let tree = TempTree::new();
    tree.skill("sample", "sample", "stable metadata", b"first body\n");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "sample");

    let first = load(&root, &entry).expect("the discovered skill should load");
    assert_eq!(first.text(), "first body\n");
    let first_digest = first.source_sha256();

    tree.write_skill_file(
        "sample",
        &tree.skill_bytes("sample", "stable metadata", b"second body\n"),
    );
    let second = load(&root, &entry).expect("a body-only edit should still load");
    assert_eq!(second.text(), "second body\n");
    assert_ne!(second.source_sha256(), first_digest);
}
