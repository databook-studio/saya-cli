//! Q04: the release documentation is a set of executable claims. This suite
//! pins only what was chosen to pin, against the real binary and the real
//! tree:
//!
//! - every slash command the 0.4.2 docs sections document appears in the
//!   binary's `/help` output (a real `--turn-file` run, isolated roots);
//! - the instruction caps are asserted through `/context`'s real output,
//!   with the expected numbers built from the config constants;
//! - the 1 KiB preference text bound is asserted through behaviour (the
//!   64-item cap is already covered by `saya-types/tests/project_preference.rs`);
//! - ADR 0009 exists, names each documented command, cites only files that
//!   exist, and states the bounds whose constants are private to `saya-cli`
//!   (each of those is guarded behaviourally by an existing unit test; see
//!   the constants' own modules). The bounds whose constants are `pub` are
//!   pinned in each owning crate's `tests/adr_bounds.rs`.
//!
//! No model and no TTY: every run is a slash command or a preferences
//! subcommand, and the provider env points at a dead address.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// The ADR this release documents; every path and number claim below hangs
/// off it.
fn adr_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/adr-0009-native-coding-and-continuity.md")
}

fn adr_text() -> String {
    std::fs::read_to_string(adr_path())
        .expect("docs/adr-0009-native-coding-and-continuity.md exists at the repo root")
}

/// The slash commands the 0.4.2 sections of docs/commands.md document.
const Q04_COMMANDS: &[&str] = &[
    "/notes",
    "/recovery",
    "/rotate",
    "/reload",
    "/setup",
    "/preferences",
    "/sessions",
    "/context",
];

struct Env {
    root: PathBuf,
    project: PathBuf,
    sessions: PathBuf,
}

/// An isolated scratch environment, shaped like the instruction-examples
/// suite: config home `root`, a fake git worktree at `root/project`, and
/// sessions/state outside the worktree.
fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!(
        "saya-docs-claims-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
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

/// The real binary in the isolated roots; the provider env points at a dead
/// address that slash-command turns never contact.
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

/// One slash-command turn via `--turn-file`.
fn slash_turn(env: &Env, name: &str, line: &str) -> std::process::Output {
    let turn = env.project.join(name);
    std::fs::write(&turn, format!("{line}\n")).unwrap();
    saya(env)
        .args([
            "--workspace",
            ".",
            "--approval-mode",
            "read-only",
            "--format",
            "ndjson",
            "--turn-file",
            turn.to_str().unwrap(),
        ])
        .output()
        .unwrap()
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

/// Every command the docs document must exist where a reader will look for
/// it: the binary's own `/help` listing.
#[test]
fn every_documented_command_appears_in_the_real_help_output() {
    for line in Q04_COMMANDS {
        let env = env("help");
        let output = slash_turn(&env, "help.txt", "/help");
        exit_zero(&output, "the /help turn");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            stdout.contains(line),
            "the real /help output must name {line}: {stdout}"
        );
        let _ = std::fs::remove_dir_all(&env.root);
    }
}

/// The instruction caps are visible in real output: `/context` prints the
/// effective limits, and the numbers must equal the config constants.
#[test]
fn context_prints_the_instruction_limits_the_constants_resolve() {
    let env = env("context-limits");
    let output = slash_turn(&env, "context.txt", "/context");
    exit_zero(&output, "the /context turn");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let limits = format!(
        "project {} B, personal {} B, total {} B",
        saya_config::DEFAULT_PROJECT_INSTRUCTION_BYTES,
        saya_config::DEFAULT_PERSONAL_INSTRUCTION_BYTES,
        saya_config::DEFAULT_TOTAL_INSTRUCTION_BYTES,
    );
    assert!(
        stdout.contains(&limits),
        "the /context settings line must state {limits}; stdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The 1 KiB preference text bound, by behaviour: 1024 bytes are admitted,
/// 1025 are refused with the usage exit code. (The 64-item cap is already
/// covered by `saya-types/tests/project_preference.rs`.)
#[test]
fn preferences_admit_1024_bytes_and_refuse_1025() {
    let env = env("preference-bound");
    let run = |args: &[&str]| {
        let mut command = saya(&env);
        command.args(args);
        command.output().unwrap()
    };
    let at_limit = "x".repeat(1024);
    let over_limit = "x".repeat(1025);
    let output = run(&["preferences", "add", &at_limit]);
    exit_zero(&output, "preferences add at exactly 1024 bytes");
    let output = run(&["preferences", "add", &over_limit]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a 1025-byte preference is refused with exit 2; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Inline code spans of the ADR — every `crates/…` and `docs/…` span must be
/// a real path in the tree, so the ADR cannot cite a file that moved.
#[test]
fn the_adr_cites_only_files_that_exist() {
    let adr = adr_text();
    let mut paths: Vec<&str> = Vec::new();
    for span in adr.split('`').skip(1).step_by(2) {
        if span.starts_with("crates/") || span.starts_with("docs/") {
            paths.push(span);
        }
    }
    assert!(
        paths.len() >= 10,
        "the ADR must cite the code it documents; found {paths:?}"
    );
    for path in paths {
        let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        assert!(
            full.exists(),
            "the ADR cites {path}, which does not exist in the tree"
        );
    }
}

/// The ADR documents the same command set the docs sections do — pinned in
/// the What-shipped table, so an edited table row fails here even where the
/// command name appears elsewhere in the prose.
#[test]
fn the_adr_names_each_documented_command() {
    let adr = adr_text();
    let start = adr
        .find("## What shipped")
        .expect("the ADR has a What shipped section");
    let end = adr[start..]
        .find("## Bounds")
        .expect("the What shipped section is followed by Bounds")
        + start;
    let shipped = &adr[start..end];
    for line in Q04_COMMANDS {
        assert!(
            shipped.contains(line),
            "the ADR's What shipped table must name {line} among what shipped"
        );
    }
}

/// Bounds whose constants are private to `saya-cli` cannot be imported by a
/// test in another crate, so the ADR's numbers for them are pinned here
/// literally; the constants themselves are guarded behaviourally by the
/// crate's own unit tests (turn_summary_tests, ops_tests, record_block_tests,
/// diff_tests, workspace_write_tests). The needle is the ADR's
/// `value | \`CONSTANT\`` table cells, so editing either side fails here.
#[test]
fn the_adr_states_the_private_bounds_their_unit_tests_guard() {
    let adr = adr_text();
    for (value, name) in [
        ("16", "MAX_REFRESHED_FILES"),
        ("32", "REFRESH_BOUND"),
        ("65536 B", "DIFF_MAX_BYTES"),
        ("2000", "DIFF_MAX_LINES"),
        ("4000000", "DIFF_MAX_CELLS"),
        ("200", "DIFF_MAX_OUTPUT_LINES"),
        ("8", "UNFINISHED_ENTRIES"),
        ("120", "ENTRY_CHARS"),
        ("65536 B", "WORKSPACE_WRITE_MAX_BYTES"),
    ] {
        let needle = format!("{value} | `{name}`");
        assert!(
            adr.contains(&needle),
            "the ADR bounds table must state `{needle}` (private constant; \
             the code value is guarded by saya-cli's unit tests)"
        );
    }
}
