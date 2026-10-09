//! Cross-adapter parity for the `/investigation` slash adapter (S9): the
//! `/investigation …` slash commands must call the *same* operation as the
//! headless `saya investigation` commands and add nothing — no second
//! parsing, no second store, no second rendering.
//!
//! Detection, not demonstration: each test parses the same intent through the
//! clap parser (in process, the authority on `InvestigationCommand`) and
//! through the slash parser, asserts the two translations are the same value,
//! and asserts the rendered output is byte-identical (the save output's
//! time-derived id hash and unix-ms stamps masked — two saves a millisecond
//! apart differ only there).
//!
//! The TUI adapter adds only the save-without-SQL fill from the last
//! selectable query; its logic is pinned by unit tests beside the adapter
//! (`tui/dispatch_investigation_tests.rs`), and the wrapper is the same
//! capture-and-push shape `dispatch_contracts` uses.

use clap::Parser as _;
use saya_cli::{
    Cli, Command, InvestigationCommand, RenderFormat, RuntimeConfig, SlashCommand,
    capture_output_start, capture_output_take, load_with_sources, parse_slash_command,
    run_investigation,
};
use saya_store::SqliteStateStore;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_slash_parity.rs; the investigations root
// is composed on the runtime (`RuntimeConfig::investigations_root`), so no
// test mutates process env and no lock is needed.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-slash-parity-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// A runtime with one sqlite profile (`local`), auto-selected as the default.
/// The profile's database is an empty file: saving validates and never
/// connects, and these tests never `run` anything. The investigations root is
/// composed on the runtime itself — no environment variable is touched.
fn runtime_at(root: &Path) -> RuntimeConfig {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    let mut runtime = load_with_sources(&options, root, root, BTreeMap::new()).unwrap();
    runtime.investigations_root = root.join("investigations");
    runtime
}

async fn store_at(root: &Path) -> SqliteStateStore {
    SqliteStateStore::new(root.join("state.sqlite3"))
}

/// Runs one `InvestigationCommand` through the shared dispatcher, capturing
/// the exact bytes a headless command would print.
async fn run_headless(
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_investigation(command, runtime, RenderFormat::Text, false, store)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}

/// The clap leg: parse the argv in process — the same parser the real `saya
/// investigation` invocation runs — and extract the subcommand.
fn clap_investigation(args: &[&str]) -> InvestigationCommand {
    let cli = Cli::try_parse_from(
        std::iter::once("saya")
            .chain(std::iter::once("investigation"))
            .chain(args.iter().copied()),
    )
    .expect("the clap argv parses");
    match cli.command {
        Some(Command::Investigation { command }) => command,
        other => panic!("expected an investigation subcommand, got {other:?}"),
    }
}

/// The slash leg: parse the line, expecting the investigation variant.
fn slash_investigation(line: &str) -> InvestigationCommand {
    match parse_slash_command(line) {
        Ok(Some(SlashCommand::Investigation(command))) => command,
        other => panic!("expected SlashCommand::Investigation for {line:?}, got {other:?}"),
    }
}

/// Masks the save-time-derived spans so two saves of the same name+SQL a
/// millisecond apart compare byte for byte elsewhere: the id's hash suffix
/// and the two unix-ms stamps. Everything else — the SQL, dialect,
/// connection, objects, name — must be identical, so a second code path that
/// drifted would still fail this comparison.
fn mask_save_volatile(output: &str) -> String {
    let mut lines = output.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut masked = vec![mask_id_text(first.trim())];
    for line in lines {
        masked.push(mask_field_line(line));
    }
    masked.join("\n")
}

/// `"recent-orders-a1b2c3d4"` → `"recent-orders-MASKED"` (the slug stays; the
/// save-time hash goes).
fn mask_id_text(id: &str) -> String {
    match id.rsplit_once('-') {
        Some((slug, hash)) if hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => {
            format!("{slug}-MASKED")
        }
        _ => id.to_string(),
    }
}

fn mask_field_line(line: &str) -> String {
    let indent = line.len() - line.trim_start().len();
    let Some((field, rest)) = line.trim_start().split_once(' ') else {
        return line.to_string();
    };
    if !matches!(
        field,
        "\"id\":" | "\"created_unix_ms\":" | "\"updated_unix_ms\":"
    ) {
        return line.to_string();
    }
    let had_comma = rest.ends_with(',');
    let value = rest.trim_end_matches(',');
    let masked = if value.chars().all(|c| c.is_ascii_digit()) {
        "<ms>".to_string()
    } else if let Some(stripped) = value.strip_prefix('"')
        && let Some(stripped) = stripped.strip_suffix('"')
    {
        mask_id_text(stripped)
    } else {
        value.to_string()
    };
    format!(
        "{}{} {}{}",
        " ".repeat(indent),
        field,
        masked,
        if had_comma { "," } else { "" }
    )
}

const SAVE_SQL: &str = "SELECT 1 AS one";

/// Saves one document through the given argv/slash pair and returns (code,
/// out, err) for the slash path.
async fn save_document(
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
) -> (InvestigationCommand, i32, String, String) {
    let clap_args = [
        "save",
        "--name",
        "Total orders",
        "--sql",
        SAVE_SQL,
        "--connection",
        "local",
    ];
    let clap_command = clap_investigation(&clap_args);
    let slash_line = "/investigation save Total orders --sql SELECT 1 AS one --connection local";
    let slash_command = slash_investigation(slash_line);
    assert_eq!(
        slash_command, clap_command,
        "the slash translation must equal the clap-parsed command"
    );
    let result = run_headless(slash_command.clone(), runtime, store).await;
    (clap_command, result.0, result.1, result.2)
}

// ---------------------------------------------------------------------------
// 1. save: slash and clap translate identically and produce the same bytes
//    (modulo the time-derived id hash and stamps).
// ---------------------------------------------------------------------------
include!("investigation_slash_parity/command_parity.rs");

// ---------------------------------------------------------------------------
// 5. A usage error never reaches the store and carries guidance.
// ---------------------------------------------------------------------------
include!("investigation_slash_parity/parser_refusals.rs");
