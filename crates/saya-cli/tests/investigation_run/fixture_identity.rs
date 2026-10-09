/// Two `cargo test` processes must never share a fixture directory: every
/// temp root this file builds carries this process's id, so a second process
/// cannot re-create or ALTER a database another is still replaying.
#[test]
fn fixture_roots_carry_this_process_id() {
    let h = harness_with_database("isolation");
    let database_root = h.database.parent().unwrap().to_path_buf();
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
