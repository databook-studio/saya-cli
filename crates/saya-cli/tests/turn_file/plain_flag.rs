use super::*;

/// `--plain` chooses which surface a launch opens; it says nothing about the
/// turn. So `--plain --turn-file <path>` must be the run `--turn-file <path>`
/// is today, byte for byte — same stdout, same stderr, same exit code. The
/// pin exists so a future surface switch cannot quietly alter the scripted
/// run's contract.
#[test]
fn plain_with_turn_file_is_byte_identical_to_turn_file_alone() {
    let env = test_root("plain-pin");
    let (address, _bodies) = mock("done");
    let path = env.root.join("instruction.md");
    // A turn that reaches no provider: the command's own rendering is the
    // whole output, so nothing but the launch surface can move it.
    std::fs::write(&path, "/help\n").unwrap();

    let run = |extra: &[&str]| {
        let mut args = base_args(&env);
        args.extend(extra.iter().map(|arg| (*arg).to_owned()));
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        let runtime_args = args.iter().map(String::as_str).collect::<Vec<_>>();
        saya(&env, &address, &runtime_args).output().unwrap()
    };

    let alone = run(&[]);
    let plain = run(&["--plain"]);

    assert_eq!(
        alone.status.code(),
        Some(0),
        "the plain run's baseline must itself pass; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&alone.stdout),
        String::from_utf8_lossy(&alone.stderr)
    );
    assert_eq!(
        plain.status.code(),
        alone.status.code(),
        "`--plain` must not move the exit code; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&plain.stdout),
        String::from_utf8_lossy(&plain.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&plain.stdout),
        String::from_utf8_lossy(&alone.stdout),
        "`--plain --turn-file` stdout must be byte-identical to `--turn-file`"
    );
    assert_eq!(
        String::from_utf8_lossy(&plain.stderr),
        String::from_utf8_lossy(&alone.stderr),
        "`--plain --turn-file` stderr must be byte-identical to `--turn-file`"
    );

    let _ = std::fs::remove_dir_all(&env.root);
}
