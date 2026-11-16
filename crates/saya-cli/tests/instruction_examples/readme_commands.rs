/// The README's preference and doctor command lines, verbatim: add the
/// README's sample text, list it, forget the id `list` printed, list again —
/// and `config doctor --task coding` against a minimal local-provider
/// config. Same worktree, same state db, cwd inside the worktree.
#[test]
fn readme_preference_and_doctor_commands_run_in_an_isolated_worktree() {
    let env = env("readme-commands");
    // The minimal config `config doctor --task coding` needs: a local
    // provider that requires no credentials.
    std::fs::write(
        env.root.join("saya/config.toml"),
        "[ai]\nprovider = 'ollama'\nmodel = 'x'\n",
    )
    .unwrap();

    let add_line = README_RUNNABLE
        .iter()
        .find(|line| line.starts_with("saya preferences add "))
        .expect("the README register carries the add line");
    let text = add_line
        .strip_prefix("saya preferences add \"")
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("the add line is `saya preferences add \"<text>\"`");

    let run = |args: &[&str]| saya(&env).args(args).output().unwrap();

    let output = run(&["preferences", "add", text]);
    exit_zero(&output, "preferences add");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("Added preference pp-"),
        "add confirms with the pp- id: {stdout}"
    );

    let output = run(&["preferences", "list"]);
    exit_zero(&output, "preferences list");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains(text),
        "list shows the added preference text: {stdout}"
    );
    let id = stdout
        .split("pp-")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .map(|tail| format!("pp-{tail}"))
        .expect("list prints the pp- id");

    let output = run(&["preferences", "forget", &id]);
    exit_zero(&output, "preferences forget");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains(&format!("Forgot preference {id}")),
        "forget names the id it removed: {stdout}"
    );

    let output = run(&["preferences", "list"]);
    exit_zero(&output, "preferences list after forget");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("No project preferences"),
        "the worktree is empty again: {stdout}"
    );

    let output = run(&["config", "doctor", "--task", "coding"]);
    exit_zero(&output, "config doctor --task coding");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("task coding: ready"),
        "the doctor reports coding readiness: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
