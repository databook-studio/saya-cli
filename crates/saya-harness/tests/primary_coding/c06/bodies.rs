use saya_harness::coding::skills::{
    MAX_SKILL_BODY_BYTES, MAX_SKILL_HEADER_BYTES, SkillLoadError, SkillOrigin,
};

use super::support::{TempTree, entry_for, load, root};

#[test]
fn round_trip_returns_exact_body_and_externally_pinned_digests() {
    let tree = TempTree::new();
    tree.skill(
        "sample",
        "sample",
        "simple demo",
        b"Body line one.\nBody line two.\n",
    );
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "sample");

    let body = load(&root, &entry).expect("skill body should load");

    assert_eq!(body.name(), "sample");
    assert_eq!(body.origin(), SkillOrigin::User);
    assert_eq!(body.trust_label(), "user");
    assert_eq!(body.text(), "Body line one.\nBody line two.\n");
    assert_eq!(body.body_len(), 30);
    // Independently computed with `shasum -a 256` over the exact bytes the
    // fixture writes, so the loader is pinned to externally derived digests
    // rather than to one of its own outputs.
    assert_eq!(
        body.source_sha256(),
        "c9a3708e285e52d6d33766fa688923a81808365aed81dec9e402d2efd1fa3344"
    );
    assert_eq!(
        entry.metadata_sha256,
        "e66d48627afbdc023509f195781e2159b642bffb2862f5ed11688897f51bb489"
    );
}

#[test]
fn body_of_exactly_the_body_cap_loads() {
    let header = b"---\nname: caps\ndescription: cap boundary\n---\n";
    let tree = TempTree::new();
    tree.skill("caps", "caps", "cap boundary", b"placeholder");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "caps");

    let mut file = header.to_vec();
    file.extend_from_slice(&[b'a'; MAX_SKILL_BODY_BYTES]);
    tree.write_skill_file("caps", &file);
    let body = load(&root, &entry).expect("a body exactly at the body cap should load");
    assert_eq!(body.body_len(), MAX_SKILL_BODY_BYTES);
    assert!(body.text().bytes().all(|byte| byte == b'a'));
}

#[test]
fn body_one_byte_over_the_body_cap_refuses_with_the_body_cap() {
    let header = b"---\nname: caps\ndescription: cap boundary\n---\n";
    let tree = TempTree::new();
    tree.skill("caps", "caps", "cap boundary", b"placeholder");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "caps");

    let mut file = header.to_vec();
    file.extend_from_slice(&[b'a'; MAX_SKILL_BODY_BYTES + 1]);
    tree.write_skill_file("caps", &file);
    let error = load(&root, &entry).expect_err("a body one byte over the cap should refuse");
    assert!(matches!(
        error,
        SkillLoadError::TooLarge { max } if max == MAX_SKILL_BODY_BYTES
    ));
}

#[test]
fn large_header_with_fitting_body_one_byte_over_the_combined_cap_refuses() {
    let cap = MAX_SKILL_HEADER_BYTES
        .checked_add(MAX_SKILL_BODY_BYTES)
        .expect("the fixed header and body caps should add without overflow");
    // A header one byte over the header cap: the body still fits its own
    // cap, so only the combined read bound can refuse this snapshot. The
    // refusal happens during the capped read, before any frontmatter parse.
    let mut header = b"---\nname: caps\ndescription: ".to_vec();
    header.resize(MAX_SKILL_HEADER_BYTES + 1 - 5, b'x');
    header.extend_from_slice(b"\n---\n");
    assert_eq!(header.len(), MAX_SKILL_HEADER_BYTES + 1);
    let tree = TempTree::new();
    tree.skill("caps", "caps", "cap boundary", b"placeholder");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "caps");

    let mut file = header;
    file.extend_from_slice(&[b'a'; MAX_SKILL_BODY_BYTES]);
    assert_eq!(file.len(), cap + 1);
    tree.write_skill_file("caps", &file);
    let error = load(&root, &entry).expect_err("one byte over the combined cap should refuse");
    assert!(matches!(error, SkillLoadError::TooLarge { max } if max == cap));
}

#[test]
fn non_utf8_bodies_are_refused_as_invalid_text() {
    let tree = TempTree::new();
    tree.skill("binary", "binary", "raw bytes", b"placeholder");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "binary");

    let mut file = tree.skill_bytes("binary", "raw bytes", b"");
    file.extend_from_slice(&[0xff, 0xfe, 0xfd]);
    tree.write_skill_file("binary", &file);
    assert!(matches!(
        load(&root, &entry),
        Err(SkillLoadError::InvalidText)
    ));
}

#[test]
fn nul_bytes_in_bodies_are_refused_as_invalid_text() {
    let tree = TempTree::new();
    tree.skill("nul", "nul", "embedded nul", b"placeholder");
    let root = root(&tree, SkillOrigin::User, "user");
    let entry = entry_for(&root, "nul");

    tree.write_skill_file(
        "nul",
        &tree.skill_bytes("nul", "embedded nul", b"before\0after\n"),
    );
    assert!(matches!(
        load(&root, &entry),
        Err(SkillLoadError::InvalidText)
    ));
}
