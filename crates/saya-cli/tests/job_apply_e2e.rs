//! End-to-end job proof through the real binary: start a managed job, let the
//! scripted model read and edit a file, run a check command in the job's
//! checkout, then review and apply the candidate back to the repository.
//!
//! This runs wherever Git is installed (Windows included): the job harness,
//! snapshot, checkout, and apply-back paths are the ones a release ships.

#[path = "job_apply_e2e/diagnostics.rs"]
mod diagnostics;
#[path = "job_apply_e2e/provider.rs"]
mod provider;
#[path = "job_apply_e2e/support.rs"]
mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

use provider::{Round, digest_from, scripted};
use support::{child_path, git, git_executable, inherit_windows_system_env, temp_root};

/// Runs the real binary in `repo`, with every piece of state under `root` —
/// outside the repository, as the job state contract requires.
fn saya(root: &Path, repo: &Path, provider: &str, args: &[&str]) -> Output {
    let path = child_path(&git_executable());
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(repo)
        .env_clear()
        .env("PATH", path)
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("SAYA_CONFIG_HOME", root.join("config"))
        .env("SAYA_SESSION_DIR", root.join("sessions"))
        .env("SAYA_STATE_DB", root.join("state/saya/state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{provider}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    inherit_windows_system_env(&mut command);
    command
        .stdin(std::process::Stdio::null())
        .output()
        .expect("saya starts")
}

fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The check command, in the platform shell's spelling: it proves the edit and
/// leaves a marker the apply step must carry back.
fn check_command() -> &'static str {
    if cfg!(windows) {
        "findstr /C:\"a + b\" src\\lib.rs >nul && echo ok>check.txt"
    } else {
        "grep -q \"a + b\" src/lib.rs && printf ok > check.txt"
    }
}

#[test]
fn a_managed_job_edits_runs_a_check_and_applies() {
    let root = temp_root("managed");
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(root.join("config/saya")).unwrap();
    std::fs::write(
        root.join("config/saya/config.toml"),
        "[job]\nroot_mode = \"managed\"\n",
    )
    .unwrap();
    git(&repo, &["init", "--quiet"]);
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .unwrap();
    git(&repo, &["add", "--", "src/lib.rs"]);
    git(&repo, &["commit", "--quiet", "-m", "baseline"]);

    let (provider, bodies) = scripted(vec![
        Box::new(|_| Round::Tool {
            name: "read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(|request| Round::Tool {
            name: "edit",
            arguments: serde_json::json!({
                "path": "src/lib.rs",
                "old_text": "a - b",
                "new_text": "a + b",
                "expected_digest": digest_from(request),
            }),
        }),
        Box::new(|_| Round::Tool {
            name: "shell",
            arguments: serde_json::json!({
                "command": check_command(),
                "description": "run the check",
                "sandbox": false,
            }),
        }),
        Box::new(|_| Round::Text("edited and checked")),
    ]);

    let started = saya(
        &root,
        &repo,
        &provider,
        &[
            "--format",
            "ndjson",
            "--approval-mode",
            "bypass",
            "start",
            "job-e2e",
            "fix the add function and run the check",
            "--write",
            ".",
            "--approve",
        ],
    );
    let (stdout, stderr) = text(&started);
    if !started.status.success() {
        let events = saya(
            &root,
            &repo,
            &provider,
            &["--format", "json", "job", "events", "job-e2e"],
        );
        let (events_out, events_err) = text(&events);
        panic!(
            "the job completes at {root:?} (repo {repo:?}): start_status={:?} stdout={stdout} stderr={stderr} events_status={:?} events={events_out} events_err={events_err}\n{}",
            started.status,
            events.status,
            diagnostics::dump(&root)
        );
    }
    let outcome = result_document(&stdout);
    assert_eq!(
        outcome["outcome"]["state"], "completed",
        "the job settled completed: {outcome}"
    );
    assert_eq!(
        outcome["outcome"]["answer"], "edited and checked",
        "the model's final text is the answer: {outcome}"
    );
    let changed: Vec<&str> = outcome["outcome"]["changed_paths"]
        .as_array()
        .expect("the outcome lists its changed paths")
        .iter()
        .map(|entry| entry["path"].as_str().expect("a changed path"))
        .collect();
    assert!(
        changed.contains(&"src/lib.rs") && changed.contains(&"check.txt"),
        "the edit and the check's marker are in the job checkout: {changed:?}\n\
         the shell tool result the model saw (last request tail): {}",
        last_request_tail(&bodies)
    );

    let reviewed = saya(
        &root,
        &repo,
        &provider,
        &[
            "--format",
            "json",
            "job",
            "review",
            "job-e2e",
            "candidate-e2e",
            "--approve",
        ],
    );
    let (review_stdout, review_stderr) = text(&reviewed);
    assert!(
        reviewed.status.success(),
        "the candidate is reviewed: stdout={review_stdout} stderr={review_stderr}\n{}",
        diagnostics::dump(&root)
    );
    let digest = source_digest(&review_stdout);

    let applied = saya(
        &root,
        &repo,
        &provider,
        &[
            "--format",
            "json",
            "job",
            "apply",
            "job-e2e",
            "candidate-e2e",
            "--source-sha256",
            &digest,
            "--approve",
        ],
    );
    let (apply_stdout, apply_stderr) = text(&applied);
    assert!(
        applied.status.success(),
        "the reviewed candidate applies: stdout={apply_stdout} stderr={apply_stderr}\n{}",
        diagnostics::dump(&root)
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("src/lib.rs")).unwrap(),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        "the edit reached the repository"
    );
    let check = std::fs::read_to_string(repo.join("check.txt")).expect("the check marker applied");
    assert!(
        check.contains("ok"),
        "the check command's marker: {check:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// A command result: the envelope carries its document as a JSON string, so
/// unwrap one layer before reading it.
fn result_document(stdout: &str) -> serde_json::Value {
    let envelope: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|_| panic!("the envelope is JSON: {stdout}"));
    let message = envelope["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the envelope carries a result message: {stdout}"));
    serde_json::from_str(message).unwrap_or_else(|_| panic!("the result is JSON: {message}"))
}

/// The last 2 KiB of the provider's last request body: the shell tool result
/// the model saw, so a failed check shows the command's own output.
fn last_request_tail(bodies: &std::sync::Mutex<Vec<String>>) -> String {
    const BOUND: usize = 2048;
    let bodies = bodies
        .lock()
        .expect("the scripted provider's bodies are readable");
    let Some(last) = bodies.last() else {
        return "<no request>".to_owned();
    };
    let mut start = last.len().saturating_sub(BOUND);
    while !last.is_char_boundary(start) {
        start += 1;
    }
    last[start..].to_owned()
}

/// The `source_sha256` the review binds the candidate to.
fn source_digest(stdout: &str) -> String {
    result_document(stdout)["source_sha256"]
        .as_str()
        .unwrap_or_else(|| panic!("the review names its source digest: {stdout}"))
        .to_owned()
}
