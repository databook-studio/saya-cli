use std::fs;

use saya_harness::coding::skills::{SkillIndexError, SkillOrigin, discover_skills};

use super::support::{TempTree, root};

#[test]
fn public_index_uses_project_precedence_and_metadata_only_digest() {
    let user = TempTree::new();
    let project = TempTree::new();
    user.skill("shared", "shared", "personal version", b"private body");
    user.skill("alpha", "alpha", "alpha text", b"user body");
    project.skill("shared", "shared", "project version", b"project body");

    let roots = [
        root(&project, SkillOrigin::Project, "project source"),
        root(&user, SkillOrigin::User, "user source"),
    ];
    let index = discover_skills(&roots).expect("valid roots should index");
    let names: Vec<_> = index
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect();
    assert_eq!(names, ["alpha", "shared"]);
    assert_eq!(index.skills[1].description, "project version");
    assert_eq!(index.skills[1].origin, SkillOrigin::Project);
    assert_eq!(index.skills[1].trust_label, "project source");

    let reversed = [
        root(&user, SkillOrigin::User, "user source"),
        root(&project, SkillOrigin::Project, "project source"),
    ];
    let reversed_index = discover_skills(&reversed).unwrap();
    assert_eq!(reversed_index.skills[1].description, "project version");
    assert_eq!(reversed_index.skills[1].origin, SkillOrigin::Project);

    let before = index.skills[1].metadata_sha256.clone();
    let header = b"---\nname: shared\ndescription: project version\n---\n";
    use sha2::{Digest, Sha256};
    let expected: String = Sha256::digest(header)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(before, expected);
    fs::write(
        project.path().join("shared/SKILL.md"),
        b"---\nname: shared\ndescription: project version\n---\nbody changed only\n",
    )
    .expect("body-only change should be written");
    let after = discover_skills(&roots).expect("changed source should still index");
    assert_eq!(after.skills[1].metadata_sha256, before);
}

#[test]
fn large_body_is_not_part_of_metadata_result_or_header_budget() {
    let project = TempTree::new();
    let skill = project.path().join("large");
    let marker = project.path().join("body-was-not-executed");
    fs::create_dir_all(&skill).unwrap();
    let mut source = b"---\nname: large\ndescription: bounded metadata\n---\n".to_vec();
    source.extend_from_slice(format!("touch {}\n", marker.display()).as_bytes());
    source.extend(std::iter::repeat_n(b'x', 9 * 1024 * 1024));
    fs::write(skill.join("SKILL.md"), source).unwrap();

    let index = discover_skills(&[root(&project, SkillOrigin::Project, "project")])
        .expect("large body must not consume header bounds");
    assert_eq!(index.skills.len(), 1);
    assert_eq!(index.skills[0].description, "bounded metadata");
    assert_eq!(index.skills[0].metadata_sha256.len(), 64);
    assert!(!marker.exists(), "listing must not execute body text");
}

#[test]
fn project_winner_does_not_hide_duplicate_user_names_in_either_root_order() {
    let user = TempTree::new();
    let project = TempTree::new();
    user.skill("first-user", "shared", "first user skill", b"");
    user.skill("second-user", "shared", "second user skill", b"");
    project.skill("project", "shared", "project winner", b"");

    let project_first = [
        root(&project, SkillOrigin::Project, "project"),
        root(&user, SkillOrigin::User, "user"),
    ];
    assert!(matches!(
        discover_skills(&project_first),
        Err(SkillIndexError::Conflict)
    ));

    let user_first = [
        root(&user, SkillOrigin::User, "user"),
        root(&project, SkillOrigin::Project, "project"),
    ];
    assert!(matches!(
        discover_skills(&user_first),
        Err(SkillIndexError::Conflict)
    ));
}
