//! `saya open` (ADR 0007, C1): one command from a local CSV file to a
//! read-only session over it. The file is staged once into a private DuckDB
//! snapshot keyed by its content hash, reused when the same content opens
//! again, previewed, and opened through a generated read-only profile — the
//! same bounded, read-only query, evidence, export, and investigation
//! machinery as any database, with no database server and no config editing.
//! `--list` and `--cleanup` manage staged snapshots.

mod connections;
mod launch;
mod manage;
mod root;
mod snapshot;
mod stage;
mod typed;

use crate::{cli::Cli, commands, render::RenderFormat};
use saya_harness::file_source::CsvStageOptions;
use std::{io::IsTerminal, path::Path};

/// The parsed `Command::Open` surface, borrowed, passed from `app::dispatch`
/// where the command short-circuits before any runtime load.
pub(crate) struct OpenInvocation<'a> {
    pub file: Option<&'a Path>,
    pub delimiter: Option<&'a str>,
    pub no_header: bool,
    pub reset: bool,
    pub typed: bool,
    pub list: bool,
    pub cleanup: Option<&'a str>,
}

pub(crate) fn run(cli: &Cli, inv: OpenInvocation<'_>) -> Result<i32, Box<dyn std::error::Error>> {
    let OpenInvocation {
        file,
        delimiter,
        no_header,
        reset,
        typed,
        list,
        cleanup,
    } = inv;
    if list && cleanup.is_some() {
        return Err("pass either --list or --cleanup, not both".into());
    }
    if (list || cleanup.is_some()) && file.is_some() {
        return Err("--list and --cleanup take no FILE argument".into());
    }
    if file.is_none() && !list && cleanup.is_none() {
        return Err("nothing to open: pass a FILE, or use --list / --cleanup".into());
    }
    if file.is_none() && (reset || typed) {
        return Err("--reset and --typed apply to opening a FILE, not to --list/--cleanup".into());
    }
    let format: RenderFormat = cli.options.format.into();
    let root = root::files_root();
    if list {
        return manage::list(&root, format);
    }
    if let Some(target) = cleanup {
        return manage::cleanup(&root, target, format);
    }
    let source = file.expect("the open path has a FILE checked above");
    let csv_only_flags = delimiter.is_some() || no_header || typed;
    if is_parquet_path(source) && csv_only_flags {
        return Err(parquet_flag_conflict(typed).into());
    }
    let options = CsvStageOptions {
        delimiter: delimiter_byte(delimiter)?,
        header: !no_header,
    };
    let session = stage::stage_or_reuse(source, &root, options, reset)?;
    if session.format == saya_harness::file_source::SourceFormat::Parquet && csv_only_flags {
        // PAR1 magic detected a Parquet file whose name says otherwise; the
        // same conflict applies, only visible after the contained read.
        return Err(parquet_flag_conflict(typed).into());
    }
    let profile = format!("file_{}", session.table);
    let typed_report = if typed {
        Some(typed::build(
            &session.db_path,
            &session.table,
            &session.preview.columns,
        )?)
    } else {
        None
    };
    let connections = connections::write_connections(&session.dir, &profile, &session.db_path)?;
    let message = launch::render_preview(&session, &connections, &profile, typed_report.as_ref());
    commands::result(message, format)?;
    if !cli.options.non_interactive && std::io::stdin().is_terminal() {
        return launch::launch(cli, &connections, &profile);
    }
    Ok(0)
}

fn delimiter_byte(arg: Option<&str>) -> Result<Option<u8>, String> {
    let Some(arg) = arg else {
        return Ok(None);
    };
    let mut chars = arg.chars();
    match (chars.next(), chars.next()) {
        (Some(single), None) if single.is_ascii() => Ok(Some(single as u8)),
        _ => Err(format!(
            "--delimiter takes exactly one ASCII character, got {arg:?}"
        )),
    }
}

/// Whether the path's extension names Parquet (a read of the path name only —
/// the content itself is only classified inside the harness's single read).
fn is_parquet_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("parquet"))
}

fn parquet_flag_conflict(typed: bool) -> String {
    if typed {
        "--typed applies to CSV sources; Parquet keeps its native column types".to_owned()
    } else {
        "this file is Parquet; --delimiter/--no-header apply to CSV sources".to_owned()
    }
}

pub(super) fn human_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub(super) fn format_time(unix_ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(unix_ms as i64)
        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unknown time".to_owned())
}
