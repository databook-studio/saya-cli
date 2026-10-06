//! The instruction-examples docs are executable claims: every `saya …`
//! command line the README shows in a ```bash block is either run against the
//! real binary here, run with a derived argument, or listed as explicitly
//! skipped with a reason — and the `/context` sample in the README is quoted
//! from real output, with the stable sentences asserted against a fresh run.
//! No model and no TTY: every turn here is a slash command or a
//! preferences/doctor subcommand, so the provider env points at a dead
//! address and is never contacted.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const README: &str = include_str!("../../../docs/instruction-examples/README.md");
const FIX_FAILING_TEST: &str =
    include_str!("../../../docs/instruction-examples/fix-failing-test.md");
const REVIEW_SQL_QUERY: &str =
    include_str!("../../../docs/instruction-examples/review-sql-query.md");
const REVIEW_DBT_MODEL: &str =
    include_str!("../../../docs/instruction-examples/review-dbt-model.md");

/// Each example file is a complete AGENTS.md body; the first non-empty line
/// doubles as the body sentinel `/context` must never leak.
const EXAMPLES: &[&str] = &[FIX_FAILING_TEST, REVIEW_SQL_QUERY, REVIEW_DBT_MODEL];

/// The README's runnable command lines, verbatim — the single source both
/// the execution test and the drift test consume. The README's ```bash
/// blocks must carry these lines byte-identically.
const README_RUNNABLE: &[&str] = &[
    "saya --workspace . --approval-mode read-only --format ndjson --turn-file ctx.txt",
    "saya preferences add \"Prefer small, focused diffs in code review\"",
    "saya preferences list",
    "saya config doctor --task coding",
];

/// README lines the tests run with one derived argument. `<id>` is the id
/// `saya preferences list` prints, so the test substitutes a real one.
const README_DERIVED: &[(&str, &str)] = &[(
    "saya preferences forget <id>",
    "<id> is printed by `saya preferences list` in the same block; the test \
     runs forget with that real id",
)];

/// README lines that need a model or a terminal; never run headless here.
const README_SKIPPED: &[(&str, &str)] = &[(
    "saya --workspace . --approval-mode bypass --format ndjson --turn-file fix.txt",
    "needs a model: the bypass edit turn drives a provider round",
)];

/// The sentences the README quotes from a real `/context` capture. The test
/// proves each appears in the binary's actual output, so the sample cannot
/// drift from the real wording.
const README_CONTEXT_SENTENCES: &[&str] = &[
    "Instruction sources for the next request",
    "Settings: project instructions on, personal instructions on; limits: project 32768 B, personal 16384 B, total 49152 B.",
    "Loaded:",
    "personal instructions.md — ",
    "project AGENTS.md — ",
    "B file, ",
    "B included, sha256 ",
    "truncated: no, redactions: 0",
    "These files carry project guidance, never permissions, and their text is never shown here.",
];

struct Env {
    root: PathBuf,
    project: PathBuf,
    sessions: PathBuf,
}

/// An isolated scratch environment: the config home is `root` (so the
/// personal instructions file is `<root>/saya/instructions.md`), the
/// worktree is `<root>/project` with a `.git` dir (preferences are scoped
/// to a canonical git worktree), and sessions/state stay outside the
/// worktree the containment guard requires.
fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!(
        "saya-instruction-examples-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("saya")).unwrap();
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    Env {
        root,
        project,
        sessions,
    }
}

/// The real binary in the isolated roots. cwd is the worktree, so `--workspace .`
/// and the preferences/doctor subcommands land on the same worktree and state.
fn saya(env: &Env) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .current_dir(&env.project)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", "http://127.0.0.1:9/v1")
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn exit_zero(output: &std::process::Output, what: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{what} must exit zero; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// One `/context` turn via the README's own command line, run verbatim from
/// the worktree with `ctx.txt` holding `/context` as its only content.
fn context_turn(env: &Env) -> std::process::Output {
    let turn = env.project.join("ctx.txt");
    std::fs::write(&turn, "/context\n").unwrap();
    saya(env)
        .args([
            "--workspace",
            ".",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "--format",
            "ndjson",
            "--turn-file",
            turn.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

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

/// The README's preference and doctor command lines, verbatim: add the
/// README's sample text, list it, forget the id `list` printed, list again —
/// and `config doctor --task coding` against a minimal local-provider
/// config. Same worktree, same state db, cwd inside the worktree.
#[test]
fn readme_preference_and_doctor_commands_run_in_an_isolated_worktree() {
    let env = env("readme-commands");
    // The minimal config `config doctor --task coding` needs: a local
    // provider that requires no credentials.
    std::fs::write(
        env.root.join("saya/config.toml"),
        "[ai]\nprovider = 'ollama'\nmodel = 'x'\n",
    )
    .unwrap();

    let add_line = README_RUNNABLE
        .iter()
        .find(|line| line.starts_with("saya preferences add "))
        .expect("the README register carries the add line");
    let text = add_line
        .strip_prefix("saya preferences add \"")
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("the add line is `saya preferences add \"<text>\"`");

    let run = |args: &[&str]| saya(&env).args(args).output().unwrap();

    let output = run(&["preferences", "add", text]);
    exit_zero(&output, "preferences add");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("Added preference pp-"),
        "add confirms with the pp- id: {stdout}"
    );

    let output = run(&["preferences", "list"]);
    exit_zero(&output, "preferences list");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains(text),
        "list shows the added preference text: {stdout}"
    );
    let id = stdout
        .split("pp-")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .map(|tail| format!("pp-{tail}"))
        .expect("list prints the pp- id");

    let output = run(&["preferences", "forget", &id]);
    exit_zero(&output, "preferences forget");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains(&format!("Forgot preference {id}")),
        "forget names the id it removed: {stdout}"
    );

    let output = run(&["preferences", "list"]);
    exit_zero(&output, "preferences list after forget");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("No project preferences"),
        "the worktree is empty again: {stdout}"
    );

    let output = run(&["config", "doctor", "--task", "coding"]);
    exit_zero(&output, "config doctor --task coding");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("task coding: ready"),
        "the doctor reports coding readiness: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Drift guard: every `saya …` line in a README ```bash block must be in one
/// of the registers above (run / derived with its reason / skipped with its
/// reason), and every register entry must still appear in the README — so a
/// changed command fails here until the register is updated too.
#[test]
fn readme_bash_lines_are_run_derived_or_explicitly_skipped() {
    let mut in_bash = false;
    let mut seen: Vec<&str> = Vec::new();
    for line in README.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("```") {
            in_bash = !in_bash && rest == "bash";
            continue;
        }
        if in_bash && trimmed.starts_with("saya ") {
            seen.push(trimmed);
        }
    }
    assert!(!seen.is_empty(), "the README must show commands");
    for line in &seen {
        let accounted = README_RUNNABLE.contains(line)
            || README_DERIVED
                .iter()
                .any(|(registered, _)| registered == line)
            || README_SKIPPED
                .iter()
                .any(|(registered, _)| registered == line);
        assert!(
            accounted,
            "README bash line {line:?} is neither run by these tests nor \
             listed with a reason in the run/derived/skipped registers"
        );
    }
    for line in README_RUNNABLE {
        assert!(
            seen.contains(line),
            "registered runnable line {line:?} is no longer in the README"
        );
    }
    for (registered, _) in README_DERIVED.iter().chain(README_SKIPPED.iter()) {
        assert!(
            seen.contains(registered),
            "registered line {registered:?} is no longer in the README"
        );
    }
}

/// The README's `/context` sample is a real capture: the README's command
/// line runs verbatim, and every quoted sentence appears in the actual
/// output — and in the README itself, so a fake quote cannot pass.
#[test]
fn readme_context_sample_matches_the_real_binary() {
    let env = env("context-sample");
    std::fs::write(
        env.project.join("AGENTS.md"),
        "Run cargo fmt before committing.\n",
    )
    .unwrap();
    std::fs::write(
        env.root.join("saya/instructions.md"),
        "Personal: keep answers short.\n",
    )
    .unwrap();
    let output = context_turn(&env);
    exit_zero(&output, "the /context turn");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    for sentence in README_CONTEXT_SENTENCES {
        assert!(
            stdout.contains(sentence),
            "the README quotes {sentence:?}; the real output must carry it: {stdout}"
        );
        assert!(
            README.contains(sentence),
            "the sentence {sentence:?} must stay quoted in the README sample"
        );
    }
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The caps the README states: a project AGENTS.md over 32 KiB is included
/// only up to the cap, marked truncated, with the whole-file size and digest
/// still reported.
#[test]
fn an_oversize_agent_md_is_reported_truncated() {
    let env = env("oversize");
    std::fs::write(env.project.join("AGENTS.md"), "x".repeat(40_000)).unwrap();
    let output = context_turn(&env);
    exit_zero(&output, "the /context turn");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let receipt = "project AGENTS.md — 40000 B file, 32768 B included, sha256 ";
    assert!(
        stdout.contains(receipt),
        "the whole-file size and the capped inclusion are both shown: {stdout}"
    );
    assert!(
        stdout.contains("truncated: yes"),
        "truncation is marked: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
