use super::*;

/// The admission check's two name-level refusals: an interpreter name is
/// refused with its own reason (the refusal list stays in force at
/// admission), and a program the config does not allow is refused —
/// `--allow` may only draw from `[jobs.runner] allow`. Both name the
/// program, the directory, and the reason.
#[test]
fn admission_refuses_interpreters_and_names_outside_the_allow() {
    let programs = temp_dir("admission-names");
    for (program, expect) in [
        ("python3", "interpreters are refused"),
        ("bench", "not declared in [jobs.runner] allow"),
    ] {
        let error = admit(
            &scope(&[program]),
            &["other".to_owned()],
            &programs,
            Duration::from_secs(300),
        )
        .expect_err("the name must refuse");
        assert!(
            error.contains(program) && error.contains(programs.display().to_string().as_str()),
            "the refusal must name the program and the directory: {error}"
        );
        assert!(
            error.contains("approved by --allow"),
            "the refusal must say the program was approved: {error}"
        );
        assert!(
            error.contains(expect),
            "`{program}`'s refusal must carry its own reason: {error}"
        );
    }
    let _ = fs::remove_dir_all(&programs);
}

/// The admission check's staged-file refusals: a symlink, a script, and a
/// missing file each refuse with the program, the directory, and the reason
/// — the shapes `validate_call` refuses per call, refused once at start.
#[cfg(unix)]
#[test]
fn admission_refuses_staged_symlinks_scripts_and_missing_files() {
    use std::os::unix::fs::symlink;

    let programs = temp_dir("admission-files");
    symlink("/bin/echo", programs.join("linked")).unwrap();
    fs::write(programs.join("scripted"), b"#!/bin/sh\n").unwrap();

    for (program, reason) in [
        ("linked", "symlink"),
        ("scripted", "script"),
        ("absent", "not in"),
    ] {
        let error = admit(
            &scope(&[program]),
            &[program.to_owned()],
            &programs,
            Duration::from_secs(300),
        )
        .expect_err("the staged shape must refuse");
        assert!(
            error.contains(program) && error.contains(programs.display().to_string().as_str()),
            "the refusal must name the program and the directory: {error}"
        );
        assert!(
            error.contains(reason),
            "`{program}`'s refusal must carry its own reason: {error}"
        );
    }
    let _ = fs::remove_dir_all(&programs);
}

/// A correctly staged binary admits: a regular, non-symlink, non-script
/// file whose name is in resolved `allow` is the one shape admission
/// approves.
#[cfg(unix)]
#[test]
fn admission_admits_a_correctly_staged_regular_file() {
    let programs = temp_dir("admission-happy");
    fs::copy("/bin/echo", programs.join("bench")).unwrap();
    admit(
        &scope(&["bench"]),
        &["bench".to_owned()],
        &programs,
        Duration::from_secs(300),
    )
    .expect("a staged regular file admits");
    let _ = fs::remove_dir_all(&programs);
}

/// The proven arm end to end: a run whose program directory sits beside the
/// roots, with the approved program staged in it, composes the sandbox,
/// probes once per run, admits, and binds the canonical directory — the
/// same directory `prepare` bound, and no other path. Also the
/// concurrent-prepare pin: a second `prepare` over the same directory both
/// proves and binds the same canonical path — sharing needs no write
/// coordination. (macOS: the platform the probe proves.)
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_proven_host_binds_the_directory_and_admits_the_staged_program() {
    let run_root = temp_dir("compose");
    fs::create_dir_all(run_root.join("workspace")).unwrap();
    fs::create_dir_all(run_root.join("state")).unwrap();
    let programs = temp_dir("compose-programs");
    fs::copy("/bin/echo", programs.join("bench")).unwrap();

    let jobs = ResolvedRunnerJobs {
        allow: vec!["bench".to_owned()],
        program_dir: Some(programs.clone()),
        ..ResolvedRunnerJobs::default()
    };
    let wiring = build(
        &jobs,
        &ResolvedInterpreterJobs::default(),
        &run_root,
        &runner_scopes(&["bench"]),
    )
    .expect("a proven host admits and binds the wiring");

    let runner = wiring.runner.expect("the probe proves this host");
    assert_eq!(
        runner.spawn.program_dir(),
        fs::canonicalize(&programs).unwrap(),
        "the tool resolves programs against the one bound directory"
    );
    assert!(
        runner.spawn.fs_roots()[0].ends_with("workspace"),
        "workspace first: the child's cwd pins to the first root"
    );
    assert!(
        wiring.plan_scopes.runner.is_some(),
        "proven: the runner scope survives plan validation"
    );

    // Concurrent prepare over the same configured directory: both prove and
    // return the same canonical program dir.
    let sandbox = saya_harness::runner::sandbox::RunSandbox::new(
        [
            fs::canonicalize(run_root.join("workspace")).unwrap(),
            fs::canonicalize(run_root.join("state")).unwrap(),
        ],
        Vec::<(String, u16)>::new(),
    )
    .expect("the roots construct");
    let first = sandbox.prepare(&programs).expect("prepare runs");
    let second = sandbox.prepare(&programs).expect("prepare runs");
    assert!(first.report().proves_runner() && second.report().proves_runner());
    assert_eq!(
        first.spawn().unwrap().program_dir(),
        second.spawn().expect("proven").program_dir(),
        "sharing the directory needs no write coordination"
    );

    let _ = fs::remove_dir_all(&run_root);
    let _ = fs::remove_dir_all(&programs);
}
