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

const README: &str = include_str!("fixtures/instruction-examples/README.md");
const FIX_FAILING_TEST: &str = include_str!("fixtures/instruction-examples/fix-failing-test.md");
const REVIEW_SQL_QUERY: &str = include_str!("fixtures/instruction-examples/review-sql-query.md");
const REVIEW_DBT_MODEL: &str = include_str!("fixtures/instruction-examples/review-dbt-model.md");

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

include!("instruction_examples/example_content.rs");

include!("instruction_examples/readme_commands.rs");

include!("instruction_examples/readme_register.rs");

include!("instruction_examples/context_receipts.rs");
