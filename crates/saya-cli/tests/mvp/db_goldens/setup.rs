//! The shared setup for the database goldens: the SQLite fixture, the REPL
//! child and its config, the redacting normaliser, and the one helper that
//! settles a scenario and hands back everything the record needs.

use std::path::{Path, PathBuf};

use super::harness;
use super::normalise::Normaliser;
use super::record::{Golden, StateRecord};

pub(super) const QUESTION: &[u8] = b"how much revenue is there\n";
pub(super) const ANSWER: &str = "the-final-answer";

/// One scenario's temp fixture: the SQLite database and the connections file
/// that names it.
pub(super) struct Fixture {
    pub(super) root: PathBuf,
    connections: PathBuf,
}

pub(super) fn fixture(tag: &str) -> Fixture {
    let root =
        std::env::temp_dir().join(format!("saya-cli-db-goldens-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("golden.sqlite");
    crate::common::sqlite_fixture(
        &database,
        &[
            "CREATE TABLE revenue (amount INTEGER);",
            "INSERT INTO revenue (amount) VALUES (10), (20);",
        ],
    );
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    Fixture { root, connections }
}

/// The one config every scenario runs: data sharing is the operator's
/// explicit consent (the provider is not a local endpoint), memory is off so
/// no post-turn extraction adds a request, and the database budget is the
/// scenario's (`[limits.database]`, `ref/05` §3: it arms on a turn's first
/// database use, so a scenario states the ceiling it stops on).
fn config(elapsed_seconds: u64) -> String {
    format!(
        "[ai]\nallow_data_sharing = true\n\n\
         [limits.database]\nelapsed_seconds = {elapsed_seconds}\n\n\
         [memory]\nmode = 'off'\n"
    )
}

pub(super) fn spawn(
    fixture: &Fixture,
    base_url: &str,
    elapsed_seconds: u64,
    tag: &str,
) -> harness::SpawnedChild {
    harness::spawn_child_with_config_extra(
        base_url,
        &config(elapsed_seconds),
        tag,
        &[
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "local",
            // The database tools require approval; a piped REPL cannot
            // answer a prompt, so the scenario states the consent itself.
            "--approval-mode",
            "bypass",
        ],
    )
}

/// The normaliser with every per-run root the child used. Built before
/// `finish`, which removes those roots.
fn normaliser(fixture: &Fixture, config_root: &Path, session_root: &Path) -> Normaliser {
    let mut normaliser = Normaliser::for_run();
    normaliser.add_root(&fixture.root);
    normaliser.add_root(config_root);
    normaliser.add_root(session_root);
    normaliser
}

/// What one settled scenario produced: the visible output, the provider's
/// request bodies and served calls, and the normaliser covering the child's
/// per-run roots.
pub(super) struct Observed {
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) bodies: Vec<String>,
    pub(super) served: Vec<harness::ServedCall>,
    pub(super) normaliser: Normaliser,
}

/// Captures the visible output, stops the child, and joins the mock. The
/// caller must have released any held response first.
pub(super) fn settle(
    fixture: &Fixture,
    mut child: harness::SpawnedChild,
    server: &mut harness::MockServer,
) -> Observed {
    let stdout = String::from_utf8_lossy(&child.stdout_buf.lock().unwrap()).into_owned();
    let stderr = String::from_utf8_lossy(&child.stderr_buf.lock().unwrap()).into_owned();
    let normaliser = normaliser(fixture, child.config_root(), child.session_root());
    child.send(b"/exit\n");
    child.finish();
    server.join();
    Observed {
        stdout,
        stderr,
        bodies: server.bodies.lock().unwrap().clone(),
        served: server.served.lock().unwrap().clone(),
        normaliser,
    }
}

/// The user-visible outcomes, in the order the turns settled. The plain
/// REPL prints an answer raw, a pending question with the pause marker, and
/// a budget stop as a diagnostic on stderr.
pub(super) fn visible_states(stdout: &str, stderr: &str) -> Vec<StateRecord> {
    let mut states = Vec::new();
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line == ANSWER {
            states.push(StateRecord {
                kind: "answer".into(),
                text: ANSWER.into(),
            });
        } else if line.starts_with("⏸ question") {
            // The question and the options printed under it are one
            // user-visible state: keep the whole block.
            let mut text = line.to_owned();
            let mut next = index + 1;
            while next < lines.len() && is_option(lines[next]) {
                text.push('\n');
                text.push_str(lines[next]);
                next += 1;
            }
            states.push(StateRecord {
                kind: "clarification".into(),
                text,
            });
            index = next;
            continue;
        }
        index += 1;
    }
    for line in stderr.lines().map(str::trim) {
        if line.contains("the run stopped on its") && line.contains("budget") {
            states.push(StateRecord {
                kind: "budget-stop".into(),
                text: line.into(),
            });
        }
    }
    states
}

/// A clarification option line: `1. option`.
fn is_option(line: &str) -> bool {
    line.split_once(". ").is_some_and(|(number, _)| {
        !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
    })
}

pub(super) fn assert_last_state(golden: &Golden, kind: &str) {
    assert_eq!(
        golden.states.last().map(|state| state.kind.as_str()),
        Some(kind),
        "the user's final state: {:?}",
        golden.states
    );
}
