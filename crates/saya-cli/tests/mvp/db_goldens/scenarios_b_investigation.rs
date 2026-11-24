//! The saved-investigation replay golden. This row is not a model loop:
//! `saya investigation run` replays saved SQL through the same read-only gate
//! as `saya query`, so the record has `requests: []` and captures the
//! statements attempted, their safety outcome, and the user-visible result.

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use super::super::classify::classify_result;
use super::super::normalise::Normaliser;
use super::super::record::{self, Golden, InvestigationRecord, StatementRecord};
use super::super::setup::fixture;

const SELECT: &str = "SELECT id, label FROM events ORDER BY id";
const WRITE: &str = "DELETE FROM events";

/// One temp root with the fixture database, a config naming the active
/// profile, and an isolated investigations directory.
struct Replay {
    root: PathBuf,
    config: PathBuf,
    investigations: PathBuf,
}

impl Replay {
    fn new(tag: &str) -> Self {
        let fixture = fixture(tag);
        let root = fixture.root;
        crate::common::sqlite_fixture(
            &root.join("golden.sqlite"),
            &[
                "CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)",
                "INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')",
            ],
        );
        let config = root.join("config.toml");
        fs::write(
            &config,
            "default_profile = 'local'\n\n[run]\nmax_rows = 50\n",
        )
        .unwrap();
        let investigations = root.join("investigations");
        fs::create_dir_all(&investigations).unwrap();
        Self {
            root,
            config,
            investigations,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = crate::common::saya_command(
            &self.root,
            &[
                "--non-interactive",
                "--format",
                "json",
                "--config",
                self.config.to_str().unwrap(),
                "--connections",
                self.root.join("connections.toml").to_str().unwrap(),
            ],
        );
        command
            .args(args)
            .env("SAYA_INVESTIGATIONS_DIR", &self.investigations)
            .output()
            .unwrap()
    }

    /// Saves the read-only investigation and returns its id.
    fn save(&self) -> String {
        let output = self.run(&[
            "investigation",
            "save",
            "--name",
            "Order events",
            "--sql",
            SELECT,
        ]);
        assert!(
            output.status.success(),
            "save failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let mut ids: Vec<String> = fs::read_dir(&self.investigations)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .map(|entry| {
                entry
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        ids.sort();
        assert_eq!(ids.len(), 1, "exactly one saved investigation");
        ids.remove(0)
    }

    /// Rewrites the saved SQL on disk, bypassing save's read-only gate, so the
    /// replay is what refuses the write.
    fn tamper(&self, id: &str) {
        let path = self.document(id);
        let mut document: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        document["sql"] = serde_json::json!(WRITE);
        fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).unwrap();
    }

    fn document(&self, id: &str) -> PathBuf {
        self.investigations.join(format!("{id}.json"))
    }
}

/// The first event a run emitted that is not the closing `result` summary:
/// `query_result` for a successful replay. A pre-execution refusal emits no
/// result event at all, only a diagnostic, so that run's kind is `error`.
fn result_kind(output: &Output) -> String {
    if let Some(kind) = stdout(output)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|event| event["event"].as_str().map(str::to_owned))
        .find(|event| event != "result")
    {
        return kind;
    }
    if stderr(output).trim().is_empty() {
        "none".into()
    } else {
        "error".into()
    }
}

/// The exact SQL a successful replay executed, from its `query_result` event.
fn executed_sql(stdout: &str) -> String {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["event"] == "query_result")
        .and_then(|event| event["result"]["executed_sql"].as_str().map(str::to_owned))
        .expect("a successful replay reports the executed SQL")
}

/// The message a refused replay reported. The refusal is a machine event on
/// stdout in JSON format; a pre-execution refusal can instead be a stderr
/// diagnostic, so both are searched.
fn error_message(output: &Output) -> String {
    let from_stdout = stdout(output);
    let from_stderr = stderr(output);
    for text in [&from_stdout, &from_stderr] {
        if let Some(message) = text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|event| event["event"] == "error")
            .and_then(|event| event["message"].as_str().map(str::to_owned))
        {
            return message;
        }
    }
    assert!(
        !from_stderr.trim().is_empty(),
        "a refused replay reports an error; stdout: {from_stdout}"
    );
    from_stderr
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_saved_investigation_replays_read_only_and_refuses_a_tampered_write() {
    let replay = Replay::new("investigation");
    let id = replay.save();

    let allowed = replay.run(&["investigation", "run", &id, "--connection", "local"]);
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "the read-only replay succeeds: {}{}",
        stdout(&allowed),
        stderr(&allowed)
    );
    let allowed_kind = result_kind(&allowed);
    assert_eq!(allowed_kind, "query_result");

    replay.tamper(&id);
    let refused = replay.run(&["investigation", "run", &id]);
    assert_eq!(
        refused.status.code(),
        Some(4),
        "the safety gate refuses the write: {}{}",
        stdout(&refused),
        stderr(&refused)
    );
    let refusal = error_message(&refused);
    assert!(
        refusal.contains("read-only safety policy"),
        "the refusal names the gate: {refusal}"
    );

    let golden = Golden::build(
        "investigation",
        &[],
        &[],
        &Normaliser::for_run(),
        Vec::new(),
    )
    .with_investigation(InvestigationRecord {
        statements: vec![
            StatementRecord {
                sql: executed_sql(&stdout(&allowed)),
                outcome: "ok".into(),
            },
            StatementRecord {
                sql: WRITE.into(),
                outcome: classify_result(&refusal).into(),
            },
        ],
        results: vec![allowed_kind, result_kind(&refused)],
    });

    assert!(golden.requests.is_empty(), "no provider request");
    let investigation = golden.investigation.as_ref().unwrap();
    assert_eq!(investigation.statements[0].sql, SELECT);
    assert_eq!(investigation.statements[1].outcome, "refused-by-safety");
    assert_eq!(investigation.results, vec!["query_result", "error"]);
    record::assert_golden("investigation", &golden);

    let _ = fs::remove_dir_all(&replay.root);
}
