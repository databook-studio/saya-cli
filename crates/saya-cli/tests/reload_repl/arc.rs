use std::{
    thread,
    time::{Duration, Instant},
};

use super::support::{DEADLINE, say, scratch_root, spawn_piped, user_config, wait_for};

/// The full piped-REPL arc: unchanged, edited, broken, restored, exit 0.
#[test]
fn reload_piped_repl_applies_edits_and_survives_a_broken_file() {
    let root = scratch_root("arc");
    std::fs::create_dir_all(user_config(&root).parent().unwrap()).unwrap();
    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();

    let (mut guard, mut stdin, streams) = spawn_piped(&root);

    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "Configuration is unchanged.",
        "the unchanged reload",
    );

    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 5\n",
    )
    .unwrap();
    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "max_rows: 100 -> 5",
        "the edited reload names the change",
    );

    std::fs::write(user_config(&root), "[run\n").unwrap();
    say(&mut stdin, "/reload");
    wait_for(&streams, "Reload failed: ", "the broken file fails");
    wait_for(
        &streams,
        "The previous configuration is still in use.",
        "the pinned sentence",
    );

    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 7\n",
    )
    .unwrap();
    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "max_rows: 5 -> 7",
        "the restored reload works again",
    );

    say(&mut stdin, "/exit");
    let mut child = guard.0.take().unwrap();
    let deadline = Instant::now() + DEADLINE;
    let code = loop {
        if let Some(code) = child.try_wait().unwrap() {
            break code.code().unwrap_or(-1);
        }
        assert!(Instant::now() < deadline, "the child never exited");
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(code, 0, "the session ends cleanly after reloads");
    let _ = std::fs::remove_dir_all(root);
}
