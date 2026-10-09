fn list_tmp_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "tmp") {
                found.push(path);
            }
        }
    }
    found
}

/// Two `cargo test` processes must never share a fixture directory: every
/// temp root this file builds carries this process's id.
#[test]
fn fixture_roots_carry_this_process_id() {
    let h = harness_with_database("isolation");
    let database_root = root_database("isolation").parent().unwrap().to_path_buf();
    let suffix = format!("-{}", std::process::id());
    assert!(
        h.root.to_str().unwrap().ends_with(&suffix),
        "the harness root must be per-process: {}",
        h.root.display()
    );
    assert!(
        database_root.to_str().unwrap().ends_with(&suffix),
        "the database fixture root must be per-process: {}",
        database_root.display()
    );
    let _ = fs::remove_dir_all(&h.root);
    let _ = fs::remove_dir_all(&database_root);
}
