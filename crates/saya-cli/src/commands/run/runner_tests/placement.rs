use super::*;

/// The composition refuses a run that approved the runner while
/// `[jobs.runner] program_dir` is undeclared: an allowlist without its
/// directory approves programs that cannot run. The refusal names the key.
#[test]
fn a_runner_approved_run_refuses_without_a_program_dir() {
    let error = build(
        &ResolvedRunnerJobs::default(),
        &ResolvedInterpreterJobs::default(),
        Path::new("/irrelevant"),
        &runner_scopes(&["bench"]),
    )
    .expect_err("no program_dir is a start-time refusal");
    assert!(
        error.contains("program_dir"),
        "the refusal must name the missing key: {error}"
    );
}

/// A run that did not approve a runner scope never consults the directory:
/// a dangling `program_dir` costs it nothing — the honest shape that keeps
/// `saya ask` and every read-only run unaffected by any state of the key.
#[test]
fn a_run_without_a_runner_scope_never_consults_the_directory() {
    let jobs = ResolvedRunnerJobs {
        program_dir: Some(temp_dir("does-not-exist-no-such-path")),
        ..ResolvedRunnerJobs::default()
    };
    let wiring = build(
        &jobs,
        &ResolvedInterpreterJobs::default(),
        Path::new("/irrelevant"),
        &Capabilities::default(),
    )
    .expect("a run without a runner scope consults nothing");
    assert!(wiring.runner.is_none());
    assert!(wiring.plan_scopes.runner.is_none());
}

/// The placement guard: the canonical program dir must not sit inside, equal
/// to, or containing any fs root — checked in both containment directions.
/// `RunSandbox::prepare` alone proves green on every one of these shapes
/// (that gap is the point), so this guard is the composition's own.
#[test]
fn the_program_dir_must_have_no_containment_relation_to_any_fs_root() {
    let root = temp_dir("place");
    let workspace = root.join("workspace");
    let state = root.join("state");
    let bin = root.join("workspace").join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&state).unwrap();
    let canonical = |path: &Path| fs::canonicalize(path).unwrap();
    let roots = [canonical(&workspace), canonical(&state)];

    for program_dir in [&bin, &workspace, &state, &root] {
        let error = place(program_dir, &roots).expect_err("the placement must refuse");
        assert!(
            error.contains("overlaps this run's filesystem root"),
            "the refusal must state the containment and its why: {error}"
        );
        assert!(
            error.contains("child write the binary the next step's run_program validates"),
            "the refusal must record why: {error}"
        );
    }

    // A sibling of the roots — the staging contract — passes.
    let sibling = temp_dir("placement-sibling");
    assert_eq!(place(&sibling, &roots).unwrap(), canonical(&sibling));
    let _ = fs::remove_dir_all(&root);
}
