use super::*;

// -- the battery: one proven provision per test, its own run dir --------

pub(super) struct Battery {
    pub(super) run: RunDir,
    pub(super) spawn: RunnerSpawn,
}

/// Builds one battery: a run dir with its `workspace/` and `state/`, the
/// staged helper — outside the roots, a child must not rewrite its own
/// allowlist — and the startup probe's verdict over the policy whose
/// first root is the run workspace (the cwd pin's requirement).
pub(super) fn battery(tag: &str, id: &str) -> Battery {
    let runs_root = leak(tag);
    let run = RunDir::create(&runs_root, &RunId::parse(id).expect("shaped run id"))
        .expect("the run dir must create");
    let workspace = run.workspace().to_path_buf();
    let state = run.state().to_path_buf();
    let programs = leak(&format!("{tag}-programs"));
    fs::copy(helper(), programs.join(NESTED_PROGRAM)).expect("the helper must stage");
    let policy = RunSandbox::new([workspace, state], Vec::<(String, u16)>::new())
        .expect("the run workspace and state construct a policy");
    let provision = policy.prepare(&programs).expect("preparation must run");
    assert!(
        provision.report().proves_runner(),
        "the healthy-host probe must prove the nested battery:\n{}",
        provision.report().render()
    );
    Battery {
        run,
        spawn: provision
            .spawn()
            .expect("the provision must be proven")
            .clone(),
    }
}
