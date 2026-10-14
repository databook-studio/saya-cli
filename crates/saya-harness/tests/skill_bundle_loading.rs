#[cfg(unix)]
#[path = "coding_runtime_contracts/skill_bundle_loading/bodies.rs"]
mod bodies;
#[cfg(unix)]
#[path = "coding_runtime_contracts/skill_bundle_loading/identity.rs"]
mod identity;
#[cfg(unix)]
#[path = "coding_runtime_contracts/skill_bundle_loading/safety.rs"]
mod safety;
#[path = "coding_runtime_contracts/skill_bundle_loading/support.rs"]
mod support;

#[cfg(not(unix))]
#[test]
fn loading_fails_closed_off_unix() {
    use saya_harness::coding::skills::{SkillEntry, SkillLoadError, SkillOrigin, load_skill_body};
    use support::{TempTree, root};

    let tree = TempTree::new();
    tree.skill("sample", "sample", "demo", b"body");
    let entry = SkillEntry {
        name: "sample".to_string(),
        description: "demo".to_string(),
        origin: SkillOrigin::User,
        trust_label: "user".to_string(),
        root_index: 0,
        relative_dir: "sample".to_string(),
        metadata_sha256: "0".repeat(64),
    };
    assert!(matches!(
        load_skill_body(&[root(&tree, SkillOrigin::User, "user")], &entry),
        Err(SkillLoadError::Unsupported)
    ));
}
