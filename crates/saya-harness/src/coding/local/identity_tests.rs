use std::{fs, os::unix::fs::symlink, path::Path};

use sha2::{Digest, Sha256};

use super::identity::{LOCAL_ROOT_ID_DOMAIN, LocalRoot, LocalRootError};
use super::test_support::Temp;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn expected_id(canonical: &Path, domain: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical.to_str().unwrap().as_bytes());
    format!("loc-{}", hex(&hasher.finalize()))
}

#[test]
fn local_root_id_is_stable_and_domain_separated() {
    let temp = Temp::new("id");
    let first_path = temp.mkdir("first");
    let second_path = temp.mkdir("second");
    let state = temp.mkdir("state");

    let first = LocalRoot::open(&first_path, &state).unwrap();
    let same = LocalRoot::open(&first_path, &state).unwrap();
    let second = LocalRoot::open(&second_path, &state).unwrap();

    assert_eq!(first.id(), same.id());
    assert_ne!(first.id(), second.id());
    assert!(first.id().as_str().starts_with("loc-"));
    assert_eq!(
        first.id().as_str(),
        expected_id(&first_path, LOCAL_ROOT_ID_DOMAIN)
    );
    // The domain tag is part of the address: the same path without it is a
    // different identity.
    assert_ne!(
        first.id().as_str(),
        expected_id(&first_path, b"saya-coding-repository-v1\0")
    );
}

#[test]
fn local_root_refuses_symlink_and_non_directory() {
    let temp = Temp::new("id-refuse");
    let directory = temp.mkdir("directory");
    let state = temp.mkdir("state");
    let file = temp.write("file", b"not a directory");

    assert!(matches!(
        LocalRoot::open(&file, &state),
        Err(LocalRootError::NotDirectory)
    ));
    assert!(matches!(
        LocalRoot::open(&temp.path().join("missing"), &state),
        Err(LocalRootError::NotFound)
    ));

    let link = temp.path().join("link");
    symlink(&directory, &link).unwrap();
    assert!(matches!(
        LocalRoot::open(&link, &state),
        Err(LocalRootError::Symlink)
    ));
}

#[test]
fn local_root_refuses_state_overlap() {
    let temp = Temp::new("id-overlap");

    let root = temp.mkdir("root");
    let state_inside_root = root.join("state");
    fs::create_dir(&state_inside_root).unwrap();
    assert!(matches!(
        LocalRoot::open(&root, &state_inside_root),
        Err(LocalRootError::StateOverlap)
    ));

    let state = temp.mkdir("outer-state");
    let root_inside_state = state.join("root");
    fs::create_dir(&root_inside_state).unwrap();
    assert!(matches!(
        LocalRoot::open(&root_inside_state, &state),
        Err(LocalRootError::StateOverlap)
    ));
}
