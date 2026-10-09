use super::*;

/// `/preferences` through the real headless binary: the slash forms drive
/// the same shared operations `saya preferences` uses, scoped to the
/// SESSION's bound worktree root (`--workspace`), never the launch cwd —
/// and the worktree label the session shows is the one the headless CLI
/// shows for the same root (one identity derivation). Add exits zero, the
/// listing carries the text with a `pp-` id, the forget removes it, and an
/// unknown subcommand exits non-zero naming the usage. One turn per run;
/// stdin is null throughout — the forms never prompt.
#[test]
fn turn_file_preferences_slash_forms_drive_the_shared_operations() {
    let env = test_root("preferences-slash");
    let (address, _bodies) = mock("unused");
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that). It holds a `.git` dir, so the CLI's walk-up
    // resolves the same canonical root the session binds explicitly.
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(workspace.join(".git")).unwrap();
    let run = |line: &str| {
        let path = env.root.join("instruction.md");
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let mut args = base_args(&env);
        args.push("--workspace".into());
        args.push(workspace.to_str().unwrap().into());
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        saya(
            &env,
            &address,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .output()
        .unwrap()
    };

    // Run 1: the add through the session surface exits zero.
    let added = run("/preferences add prefer tabs over spaces");
    assert_eq!(
        added.status.code(),
        Some(0),
        "the add completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&added.stdout),
        String::from_utf8_lossy(&added.stderr)
    );
    let added_out = String::from_utf8_lossy(&added.stdout).into_owned();
    assert!(
        added_out.contains("Added preference pp-"),
        "the add names the id: {added_out}"
    );

    // Run 2: the listing shows the text, a `pp-` id, and the same
    // worktree label the headless CLI shows for the same root.
    let listing = run("/preferences list");
    assert_eq!(
        listing.status.code(),
        Some(0),
        "the listing completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&listing.stdout),
        String::from_utf8_lossy(&listing.stderr)
    );
    let listing_out = String::from_utf8_lossy(&listing.stdout).into_owned();
    assert!(
        listing_out.contains("prefer tabs over spaces"),
        "the preference is listed: {listing_out}"
    );
    // The ndjson `result` event's message is the listing itself, unescaped
    // — tokens are split on the real message, not on the JSON encoding.
    let event_message = |stdout: &str| -> String {
        stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|value| {
                value
                    .get("message")
                    .and_then(|message| message.as_str().map(str::to_owned))
            })
            .next()
            .unwrap_or_default()
    };
    let listing_message = event_message(&listing_out);
    let session_id = listing_message
        .split_whitespace()
        .find(|token| token.len() == 35 && token.starts_with("pp-"))
        .expect("the listing carries a pp- id")
        .to_owned();
    let session_label =
        first_worktree_label(&listing_message).expect("the listing names the worktree");
    let cli_listing = {
        // The headless subcommand from the same workspace cwd, against the
        // same state db the `saya` helper isolates into the scratch root.
        let output = Command::new(env!("CARGO_BIN_EXE_saya"))
            .args(["preferences", "list"])
            .current_dir(&workspace)
            .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
            .env("SAYA_CONFIG_HOME", &env.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "the CLI listing completes: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let cli_label = first_worktree_label(&cli_listing).expect("the CLI listing names the worktree");
    assert_eq!(
        session_label, cli_label,
        "one identity derivation: the session and the CLI name the same worktree"
    );

    // Run 3: the forget removes it; the listing is empty again.
    let forgotten = run(&format!("/preferences forget {session_id}"));
    assert_eq!(
        forgotten.status.code(),
        Some(0),
        "the forget completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&forgotten.stdout),
        String::from_utf8_lossy(&forgotten.stderr)
    );
    assert!(
        String::from_utf8_lossy(&forgotten.stdout).contains("Forgot preference"),
        "{}",
        String::from_utf8_lossy(&forgotten.stdout)
    );
    let empty = run("/preferences list");
    assert!(
        String::from_utf8_lossy(&empty.stdout).contains("No project preferences for"),
        "{}",
        String::from_utf8_lossy(&empty.stdout)
    );

    // An unknown subcommand exits non-zero naming the usage.
    let unknown = run("/preferences nope");
    assert_ne!(
        unknown.status.code(),
        Some(0),
        "an unknown subcommand is a usage error; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&unknown.stdout),
        String::from_utf8_lossy(&unknown.stderr)
    );
    let stderr = String::from_utf8_lossy(&unknown.stderr).into_owned();
    assert!(
        stderr.contains("unknown /preferences subcommand: nope"),
        "the refusal names the offence and the usage: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// `/notes` through the real headless binary: bare `/notes` says there are
/// no session notes (exit zero, the remedy named), and `/notes set <text>`
/// refuses with the /compact-first remedy and exits non-zero — a
/// continuation record exists only after a compaction, and `--turn-file`
/// runs a fresh session per turn on the same session dir, so both runs are
/// the empty/refusal shapes.
#[test]
fn turn_file_notes_show_and_set_refuse_without_a_continuation() {
    let env = test_root("notes-slash");
    let (address, _bodies) = mock("unused");
    let run = |line: &str| {
        let path = env.root.join("instruction.md");
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let mut args = base_args(&env);
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        saya(
            &env,
            &address,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .output()
        .unwrap()
    };

    // Bare `/notes`: exit zero, the no-notes words with the remedy.
    let shown = run("/notes");
    assert_eq!(
        shown.status.code(),
        Some(0),
        "the show completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&shown.stdout),
        String::from_utf8_lossy(&shown.stderr)
    );
    let stdout = String::from_utf8_lossy(&shown.stdout).into_owned();
    assert!(
        stdout.contains("No session notes") && stdout.contains("/notes set <text>"),
        "the show names the remedy: {stdout}"
    );

    // `/notes set x`: exit non-zero, the /compact-first refusal.
    let refused = run("/notes set remember the failing query");
    assert_ne!(
        refused.status.code(),
        Some(0),
        "a set with no continuation record must exit non-zero; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        stderr.contains("run /compact first"),
        "the refusal names the /compact remedy: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
