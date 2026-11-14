#![cfg(unix)]

use std::{env, fs, path::PathBuf};

#[cfg(unix)]
#[test]
fn installed_homebrew_podman_is_trusted_when_opted_in() {
    let Some(path) = env::var_os("SAYA_TEST_HOME_BREW_PODMAN_PATH") else {
        return;
    };
    let path = PathBuf::from(path);
    assert_eq!(path, PathBuf::from("/opt/homebrew/bin/podman"));
    assert!(
        super::trusted_executable(&path).is_some(),
        "the installed fixed Homebrew engine and its canonical target are trusted"
    );
}

#[cfg(unix)]
#[test]
fn group_or_world_writable_directories_outside_homebrew_are_not_trusted() {
    use std::{
        os::unix::fs::PermissionsExt as _,
        time::{SystemTime, UNIX_EPOCH},
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = fs::canonicalize(env::temp_dir())
        .expect("temporary root is canonical")
        .join(format!("saya-podman-trust-{nonce}"));
    fs::create_dir(&root).expect("private test root is created");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("private test root mode is set");
    assert!(super::trusted_directories(&root));

    for (name, mode) in [("group-write", 0o770), ("world-write", 0o777)] {
        let directory = root.join(name);
        fs::create_dir(&directory).expect("test directory is created");
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode))
            .expect("test directory mode is set");
        assert!(
            !super::trusted_directories(&directory),
            "writable directory {name} outside the Homebrew prefix is not trusted"
        );
    }

    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("test root is private for cleanup");
    fs::remove_dir_all(root).expect("test-owned root is removed");
}
