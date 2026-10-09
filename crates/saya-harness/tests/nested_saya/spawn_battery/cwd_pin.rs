use super::*;

/// The parent's cwd carries `.saya/connections.toml` — the file
/// `saya-cli/src/config/sources.rs:17-25` auto-discovers from the
/// current directory. The child's cwd is the run workspace, so its own
/// discovery finds nothing; and the sandbox denies the bait by path
/// outright. Both closures, not either. The parent's cwd is made real
/// by moving the test process into the planted directory for the
/// spawn — a regressed pin would land the child there, and the bait
/// would be discovered.
#[tokio::test]
async fn the_child_cwd_is_the_workspace_and_the_parents_connections_are_not_discovered() {
    // The parent's cwd, with the auto-discovery bait planted in it.
    let parent = leak("parent-cwd");
    let bait_dir = parent.join(".saya");
    fs::create_dir_all(&bait_dir).expect("the planted .saya dir");
    let bait = bait_dir.join("connections.toml");
    fs::write(
        &bait,
        format!("[profiles.\"{PRODUCTION_PROFILE}\"]\npath = \"../../prod.db\"\n"),
    )
    .expect("the bait must plant");
    assert!(
        bait.is_file(),
        "the bait must exist for the negative to be meaningful"
    );

    let battery = battery("cwd", "cwd-1");
    let state = nested_saya::prepare(
        &battery.run,
        &[endpoint("orchestrator", None)],
        &Default::default(),
        &[corpus("corp", "workspace/corpus/corp.db")],
    )
    .expect("the generated configs");
    // The parent's cwd is real at spawn time: without the pin the child
    // would land here and discover the bait.
    struct CwdGuard(PathBuf);
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).expect("restore cwd");
        }
    }
    let guard = CwdGuard(std::env::current_dir().expect("the test process has a cwd"));
    std::env::set_current_dir(&parent).expect("chdir into the planted parent cwd");

    let resolver = StaticCredentialSource::new(Vec::<(String, String)>::new());
    let cancellation = CancellationToken::new();
    let outcome = nested_saya::ask(
        &battery.spawn,
        &scope(),
        &state,
        NestedAsk {
            profile: PROFILE,
            // The question doubles as the probe path: the bait itself.
            question: &bait.display().to_string(),
            credentials: &[],
            resolver: &resolver,
        },
        DEFAULT_TIMEOUT,
        &cancellation,
    )
    .await
    .expect("the nested ask must run");
    drop(guard);
    assert_eq!(outcome.exit_code, Some(0), "the child must have run");

    let report = &outcome.stdout.text;
    let workspace = battery.run.workspace().display().to_string();
    assert!(
        report.contains(&format!("cwd={workspace}")),
        "the child's cwd is the run workspace, not the parent's: {report}"
    );
    assert!(
        report.contains(&format!(
            "discover:{}:missing",
            battery
                .run
                .workspace()
                .join(".saya/connections.toml")
                .display()
        )),
        "the child's own discovery scan of its cwd finds nothing: {report}"
    );
    assert!(
        report.contains(&format!("open:probe:{}:denied(", bait.display())),
        "the sandbox denies the bait by path outright: {report}"
    );
    assert!(
        !report.contains(PRODUCTION_PROFILE),
        "the bait's production profile is not in the child's world: {report}"
    );

    let _ = fs::remove_dir_all(battery.run.root());
    let _ = fs::remove_dir_all(&parent);
}
