//! The retired `saya coding` spelling.
//!
//! The retired name still parses so the answer can name the spelling that
//! works for each verb, rather than clap's "unrecognized subcommand"; it stays
//! out of `--help`, and every spelling it names really runs.

fn saya(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .output()
        .unwrap()
}

fn refusal(args: &[&str]) -> String {
    let output = saya(args);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{args:?} is a usage error: {output:?}"
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn saya_coding_start_names_saya_start() {
    let stderr = refusal(&["coding", "start", "job-a", "a task", "--approve"]);
    assert!(
        stderr.contains("saya start"),
        "the moved start names `saya start`: {stderr}"
    );
}

#[test]
fn saya_coding_review_names_saya_job_review() {
    let stderr = refusal(&["coding", "review", "job-a", "checkout-a", "--approve"]);
    assert!(
        stderr.contains("saya job review"),
        "the moved lifecycle verb names `saya job review`: {stderr}"
    );
}

#[test]
fn saya_coding_approve_names_the_session_command() {
    let stderr = refusal(&["coding", "approve", "job-a"]);
    assert!(
        stderr.contains("/approve"),
        "the session-only control names `/approve`: {stderr}"
    );
}
