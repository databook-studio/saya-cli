//! The `saya job` command's rule, on the real binary: a session control typed
//! under `saya job` is a usage error naming the spelling that does work,
//! never a silently accepted no-op.
//!
//! The command shares the one `CodingCommand` grammar, so these verbs *parse*
//! here; the refusal is what keeps the shared grammar from advertising a
//! control the command cannot run.

/// Each case: the words after `saya`, and the spelling the refusal must name.
const CASES: &[(&[&str], &str)] = &[
    (
        &["job", "start", "job-a", "a task", "--approve"],
        "saya start",
    ),
    (
        &[
            "job",
            "follow-up",
            "job-a",
            "agent-a",
            "--expected-completion-sequence",
            "4",
            "check it",
            "--approve",
        ],
        "saya follow-up",
    ),
    (&["job", "approve", "job-a"], "/approve"),
    (&["job", "deny", "job-a"], "/deny"),
    (&["job", "message", "job-a", "agent-a", "hello"], "/message"),
    (&["job", "pause", "job-a", "agent-a"], "/pause"),
    (&["job", "continue", "job-a", "agent-a"], "/continue"),
    (
        &["job", "cancel-child", "job-a", "agent-a"],
        "/cancel-child",
    ),
];

#[test]
fn saya_job_refuses_a_session_control_naming_the_spelling_that_works() {
    for (argv, spelling) in CASES {
        let root =
            std::env::temp_dir().join(format!("saya-job-lane-{}-{}", std::process::id(), argv[1]));
        std::fs::create_dir_all(&root).unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .args(*argv)
            .env("HOME", &root)
            .env("SAYA_CONFIG_HOME", root.join("user-config"))
            .env("SAYA_STATE_DB", root.join("state.sqlite3"))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{argv:?} is refused as a usage error: {output:?}"
        );
        assert!(
            stderr.contains(spelling),
            "{argv:?} names `{spelling}`: {stderr}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
