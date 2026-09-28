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
// comes from `SAYA_INVESTIGATIONS_DIR`, so the env lock is mandatory (recipe:
// tests/run_slash_parity.rs).
// ---------------------------------------------------------------------------

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

/// Points `SAYA_INVESTIGATIONS_DIR` at the test's private root, returning the
/// previous value for restore. SAFETY: the caller holds `ENV_LOCK` for the
/// whole test body, so no other test in this binary observes a torn or
/// foreign investigations root.
unsafe fn set_investigations_dir(path: &Path) -> Option<std::ffi::OsString> {
    let previous = std::env::var_os("SAYA_INVESTIGATIONS_DIR");
    // SAFETY: see above; the lock is held across the whole test body.
    unsafe { std::env::set_var("SAYA_INVESTIGATIONS_DIR", path) };
    previous
}

/// SAFETY: see [`set_investigations_dir`]; the caller still holds `ENV_LOCK`.
unsafe fn restore_investigations_dir(previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => {
            // SAFETY: same lock discipline as `set_investigations_dir`.
            unsafe { std::env::set_var("SAYA_INVESTIGATIONS_DIR", value) }
        }
        None => {
            // SAFETY: same lock discipline as `set_investigations_dir`.
            unsafe { std::env::remove_var("SAYA_INVESTIGATIONS_DIR") }
        }
    }
}

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
/// connects, and these tests never `run` anything.
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
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
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
#[tokio::test]
async fn save_slash_and_clap_agree_on_the_command_and_the_bytes() {
    let _env = lock_env().await;
    let root = temp_root("save_parity");
    unsafe { set_investigations_dir(&root.join("investigations")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_clap, slash_code, slash_out, slash_err) = save_document(&runtime, &store).await;
    assert_eq!(slash_code, 0, "save stderr: {slash_err}");
    let clap_saved = run_headless(_clap.clone(), &runtime, &store).await;
    assert_eq!(clap_saved.0, 0, "clap save stderr: {}", clap_saved.2);

    assert_eq!(
        mask_save_volatile(&slash_out),
        mask_save_volatile(&clap_saved.1),
        "slash save diverged from clap save"
    );
    assert_eq!(slash_err, clap_saved.2);

    // The document is really there: list names it.
    let list = run_headless(
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
        &store,
    )
    .await;
    assert!(list.1.contains("Total orders"), "list out: {}", list.1);

    unsafe { restore_investigations_dir(None) };
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. list and show: byte for byte.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn list_and_show_slash_and_clap_agree_byte_for_byte() {
    let _env = lock_env().await;
    let root = temp_root("list_show_parity");
    unsafe { set_investigations_dir(&root.join("investigations")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, code, out, err) = save_document(&runtime, &store).await;
    assert_eq!(code, 0, "seed save stderr: {err}");
    let id = out.lines().next().unwrap().to_string();

    let clap_list = run_headless(clap_investigation(&["list"]), &runtime, &store).await;
    let slash_list =
        run_headless(slash_investigation("/investigation list"), &runtime, &store).await;
    assert_eq!(
        slash_list.1, clap_list.1,
        "slash list diverged from clap list"
    );
    assert_eq!(slash_list.2, clap_list.2);

    let clap_show = run_headless(clap_investigation(&["show", &id]), &runtime, &store).await;
    let slash_show = run_headless(
        slash_investigation(&format!("/investigation show {id}")),
        &runtime,
        &store,
    )
    .await;
    assert_eq!(slash_show.0, 0, "show stderr: {}", slash_show.2);
    assert_eq!(
        slash_show.1, clap_show.1,
        "slash show diverged from clap show"
    );
    assert_eq!(slash_show.2, clap_show.2);

    unsafe { restore_investigations_dir(None) };
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. delete: byte for byte, and the document is really gone.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn delete_slash_and_clap_agree_byte_for_byte() {
    let _env = lock_env().await;
    let root = temp_root("delete_parity");
    unsafe { set_investigations_dir(&root.join("investigations")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, code, out, err) = save_document(&runtime, &store).await;
    assert_eq!(code, 0, "seed save stderr: {err}");
    let id = out.lines().next().unwrap().to_string();

    let slash_delete = run_headless(
        slash_investigation(&format!("/investigation delete {id}")),
        &runtime,
        &store,
    )
    .await;
    assert_eq!(slash_delete.0, 0, "delete stderr: {}", slash_delete.2);

    // The clap delete of the (now gone) id must fail exactly as a slash
    // delete of it would — same class, same bytes.
    let clap_delete = run_headless(clap_investigation(&["delete", &id]), &runtime, &store).await;
    let slash_delete_again = run_headless(
        slash_investigation(&format!("/investigation delete {id}")),
        &runtime,
        &store,
    )
    .await;
    assert_ne!(clap_delete.0, 0, "deleting a missing id must fail");
    assert_eq!(
        slash_delete_again.1, clap_delete.1,
        "slash delete diverged from clap delete"
    );
    assert_eq!(slash_delete_again.2, clap_delete.2);

    // And the deleted document no longer lists.
    let list = run_headless(
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
        &store,
    )
    .await;
    assert!(
        list.1.contains("No saved investigations."),
        "the deleted document must be gone: {}",
        list.1
    );

    unsafe { restore_investigations_dir(None) };
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. `/investigations` is `list`, producing the same bytes.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn investigations_alias_output_equals_investigation_list() {
    let _env = lock_env().await;
    let root = temp_root("alias_parity");
    unsafe { set_investigations_dir(&root.join("investigations")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, code, out, err) = save_document(&runtime, &store).await;
    assert_eq!(code, 0, "seed save stderr: {err}");
    let id = out.lines().next().unwrap().to_string();

    let alias = run_headless(slash_investigation("/investigations"), &runtime, &store).await;
    let list = run_headless(slash_investigation("/investigation list"), &runtime, &store).await;
    assert_eq!(alias.0, 0, "alias stderr: {}", alias.2);
    assert_eq!(
        alias.1, list.1,
        "/investigations diverged from /investigation list"
    );
    assert!(
        alias.1.contains(&id),
        "the seeded id is listed: {}",
        alias.1
    );

    unsafe { restore_investigations_dir(None) };
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 5. A usage error never reaches the store and carries guidance.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unknown_subcommand_is_a_parse_error() {
    let parsed = parse_slash_command("/investigation bogus");
    assert!(
        parsed.is_err(),
        "an unknown subcommand must be a usage error, got {parsed:?}"
    );
    let message = parsed.unwrap_err().to_string();
    assert!(
        message.contains("usage"),
        "the usage error carries guidance: {message}"
    );
}

// ---------------------------------------------------------------------------
// 6. The headless REPL has no last query: a save with neither --sql nor
//    --file refuses there (S9 invariant 3), never falling through to the
//    operation's stdin read — stdin is the REPL's own input.
// ---------------------------------------------------------------------------
#[test]
fn headless_save_without_sql_refuses_instead_of_reading_stdin() {
    let root = temp_root("headless_save_refusal");
    let investigations = root.join("investigations");
    let config = root.join("config.toml");
    fs::write(&config, "[run]\nmax_rows = 1\n").unwrap();
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

    // One verbatim turn via --turn-file: the same per-turn entry the piped
    // loop uses, with an exit code that reflects the turn's outcome (an
    // errored turn exits 5).
    let turn = root.join("turn.txt");
    fs::write(&turn, "/investigation save Total orders\n").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--format",
            "json",
            "--turn-file",
            turn.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
        ])
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("SAYA_INVESTIGATIONS_DIR", &investigations)
        .env("SAYA_CONFIG_HOME", root.join("config-home"))
        .env("HOME", root.join("home"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.status.code(),
        Some(5),
        "the refusal must end the turn errored: {stdout}{stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("--sql") && combined.contains("--file"),
        "the refusal names both remedies: {combined}"
    );
    // Nothing was saved.
    assert!(
        fs::read_dir(&investigations)
            .map(|entries| entries.count())
            .unwrap_or(0)
            == 0,
        "no document was written: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}
