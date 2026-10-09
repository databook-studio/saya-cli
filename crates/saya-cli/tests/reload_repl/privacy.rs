use super::support::{say, scratch_root, spawn_piped, user_config, wait_for};

/// The privacy regression at the real surface: a config that widens
/// `allow_data_sharing` is reloaded but NOT applied — the plan lists it
/// under `Not applied in this session:` and a following `/privacy` still
/// reports the session's unchanged value (`off`).
#[test]
fn reload_piped_repl_never_widens_sharing_and_privacy_says_so() {
    let root = scratch_root("privacy");
    std::fs::create_dir_all(user_config(&root).parent().unwrap()).unwrap();
    std::fs::write(user_config(&root), "[ai]\nallow_data_sharing = false\n").unwrap();

    let (_guard, mut stdin, streams) = spawn_piped(&root);

    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "Configuration is unchanged.",
        "the unchanged reload",
    );

    std::fs::write(user_config(&root), "[ai]\nallow_data_sharing = true\n").unwrap();
    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "Not applied in this session:",
        "the widen is classified pending",
    );
    {
        let guard = streams.stdout.lock().unwrap();
        let stdout = String::from_utf8_lossy(&guard);
        assert!(
            stdout.contains("allow_data_sharing: false -> true"),
            "the plan names the widen: {stdout}"
        );
    }

    say(&mut stdin, "/privacy");
    wait_for(
        &streams,
        "Cloud data sharing: off",
        "/privacy still says off",
    );
    let _ = std::fs::remove_dir_all(root);
}
