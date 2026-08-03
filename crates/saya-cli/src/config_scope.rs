use std::path::{Path, PathBuf};

pub(crate) fn resolve(selected_connections: Option<&PathBuf>, cwd: &Path) -> PathBuf {
    let path = selected_connections
        .map(|path| {
            cwd.join(path)
                .canonicalize()
                .unwrap_or_else(|_| cwd.join(path))
        })
        .unwrap_or_else(|| cwd.to_path_buf());
    path.canonicalize().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    #[test]
    fn selected_connection_files_define_distinct_canonical_scopes() {
        let root = std::env::temp_dir().join(format!("saya-scope-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        fs::write(root.join("one/connections.toml"), "").unwrap();
        fs::write(root.join("two/connections.toml"), "").unwrap();
        let one = super::resolve(Some(&PathBuf::from("one/connections.toml")), &root);
        let two = super::resolve(Some(&PathBuf::from("two/connections.toml")), &root);
        assert_ne!(one, two);
        let _ = fs::remove_dir_all(root);
    }
}
