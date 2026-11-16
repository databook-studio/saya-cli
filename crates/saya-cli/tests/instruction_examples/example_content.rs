/// For every example: copy the file's whole body to the worktree's
/// `AGENTS.md`, run the README's `/context` command line, and check the
/// receipt — the source admitted whole (size == included, not truncated) and
/// the body text absent from the output.
#[test]
fn each_example_loads_and_context_reports_it_without_its_body() {
    for body in EXAMPLES {
        let env = env("example-loads");
        std::fs::write(env.project.join("AGENTS.md"), body).unwrap();
        let output = context_turn(&env);
        exit_zero(&output, "the /context turn");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let len = body.len();
        let receipt = format!("project AGENTS.md — {len} B file, {len} B included, sha256 ");
        assert!(
            stdout.contains(&receipt),
            "the whole example must be admitted (size == included): {stdout}"
        );
        assert!(
            stdout.contains("truncated: no"),
            "the example fits the project cap, so truncation is off: {stdout}"
        );
        let sentinel = body
            .lines()
            .find(|line| !line.trim().is_empty())
            .expect("an example is not blank");
        assert!(
            !stdout.contains(sentinel),
            "/context never prints bodies; the example's first line leaked: {stdout}"
        );
        let _ = std::fs::remove_dir_all(&env.root);
    }
}

/// The README promises small copy-ready examples; each example must sit far
/// below the 32 KiB project cap so a user's own additions still fit.
#[test]
fn each_example_is_far_below_the_project_cap() {
    for body in EXAMPLES {
        assert!(
            body.len() < 8 * 1024,
            "an example is {} bytes; the README promises room to spare under the 32 KiB cap",
            body.len()
        );
    }
}
