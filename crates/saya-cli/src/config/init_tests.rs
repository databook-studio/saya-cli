use super::create_project_files_with;
use std::{fs, io};

#[test]
fn rollback_removes_the_first_file_when_the_second_write_fails() {
    let root =
        std::env::temp_dir().join(format!("saya-config-init-rollback-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();

    let result = create_project_files_with(&root, |path, contents| {
        if path
            .file_name()
            .is_some_and(|name| name == "connections.toml")
        {
            return Err(io::Error::other("injected second-file failure"));
        }
        fs::write(path, contents)
    });

    assert!(result.is_err());
    assert!(!root.join(".saya/config.toml").exists());
    assert!(!root.join(".saya").exists());
    fs::remove_dir_all(root).unwrap();
}
