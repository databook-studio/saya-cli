//! Tests for the composed runtime: the investigations-root resolution and
//! the approval-mode vocabulary.

use super::*;

/// The pure resolver prefers the env override, then sits beside the
/// state database; a parentless state db falls back to a relative root.
#[test]
fn investigations_root_prefers_env_then_the_state_parent() {
    assert_eq!(
        investigations_root_from(
            Some(OsStr::new("/tmp/inv")),
            Path::new("/data/saya/state.sqlite3")
        ),
        PathBuf::from("/tmp/inv")
    );
    assert_eq!(
        investigations_root_from(None, Path::new("/data/saya/state.sqlite3")),
        PathBuf::from("/data/saya/investigations")
    );
    assert_eq!(
        investigations_root_from(None, Path::new("state.sqlite3")),
        PathBuf::from("investigations")
    );
}

/// The name map carries every mode the grammar parses, and each name
/// round-trips through the parser — the maps and `FromStr` cannot drift,
/// so a mode the flag accepts is a mode the status bar, the session
/// record, and `/approvals` can all name.
#[test]
fn the_approval_name_map_covers_the_whole_vocabulary() {
    for (value, name) in [
        ("ask", "ask"),
        ("read-only", "read-only"),
        ("never", "never"),
        ("bypass", "bypass"),
    ] {
        let options = GlobalOptions {
            approval_mode: Some(value.to_string()),
            ..Default::default()
        };
        assert_eq!(approval_name(&options).unwrap(), name);
        assert_eq!(
            approval_name(&options)
                .unwrap()
                .parse::<saya_agent::ApprovalPolicy>()
                .unwrap(),
            approval_mode(&options).unwrap(),
            "the name re-parses to the same mode: the vocabulary round-trips"
        );
    }
}

/// The bypass gate is the config's `[permissions] allow_bypass`, read once
/// here and carried on the session. A record that says `bypass` degrades to
/// `ask` when the key is unset, and the project layer can never set it — so
/// the same resume degrades inside a repository that asks for the consent.
#[test]
fn a_resumed_bypass_session_degrades_unless_the_config_allows_it() {
    let root = temp_investigations_root().join("allow-bypass");
    let user = root.join("user-config");
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".saya")).expect("temp tree");
    std::fs::create_dir_all(&user).expect("temp tree");
    std::fs::write(
        project.join(".saya/config.toml"),
        "[permissions]\nallow_bypass = true\n",
    )
    .expect("project fixture");
    let resume = |allow: bool| {
        crate::interactive::session::permissions::mode::launch_posture(
            "bypass", "build", false, allow,
        )
        .0
    };

    let runtime = load_with_sources(&GlobalOptions::default(), &project, &user, BTreeMap::new())
        .expect("a project-layer attempt is not a load failure");
    assert!(
        !allow_bypass(&runtime),
        "a project's `.saya/config.toml` cannot allow bypass"
    );
    assert_eq!(
        resume(allow_bypass(&runtime)),
        "ask",
        "the withheld mode is not restored by the record"
    );

    std::fs::write(
        user.join("config.toml"),
        "[permissions]\nallow_bypass = true\n",
    )
    .expect("user fixture");
    let runtime = load_with_sources(&GlobalOptions::default(), &project, &user, BTreeMap::new())
        .expect("the user layer resolves");
    assert!(allow_bypass(&runtime), "the user's own consent is read");
    assert_eq!(resume(allow_bypass(&runtime)), "bypass");
}

#[test]
fn missing_source_directories_still_load_defaults_without_creating_them() {
    let missing = temp_investigations_root().join("future-project");
    assert!(!missing.exists());
    let runtime = load_with_sources(
        &GlobalOptions::default(),
        &missing,
        &missing.join("user-config"),
        BTreeMap::new(),
    )
    .expect("a future source directory must not prevent default configuration");
    assert!(runtime.connections.profiles.is_empty());
    assert!(runtime.connections_path.is_none());
    assert!(
        !missing.exists(),
        "loading defaults must not create the source"
    );
}
