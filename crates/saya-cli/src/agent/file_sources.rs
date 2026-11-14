//! Whether an agent surface should tell the model how to analyse local data
//! files through the shell.
//!
//! One operation, two adapters: a coding job's checkout and an interactive
//! session both ask this module. The decision is a property of the tree and
//! the host, taken once at run or session start: the root holds at least one
//! data file (the searches' own bounded presence check, one extension at a
//! time, under its named entry and deadline bounds), and a `duckdb` executable
//! is on `PATH` (a bounded, metadata-only lookup — nothing is executed and no
//! network is touched).

use std::ffi::OsStr;

use saya_harness::workspace::{PresenceBounds, Workspace};

/// The data-file extensions the shell route covers, without the dot.
///
/// `json` is deliberately absent: every code repository holds
/// `package.json`/`tsconfig.json`, so a block keyed on it would fire almost
/// everywhere. `ndjson` and `jsonl` are line-delimited data and stay.
const DATA_EXTENSIONS: &[&str] = &["csv", "tsv", "parquet", "ndjson", "jsonl", "duckdb"];

/// The most `PATH` entries one detection reads. The lookup is metadata-only
/// and stops at the first `duckdb`; this keeps a pathological `PATH` bounded.
const PATH_ENTRY_LIMIT: usize = 64;

/// The shell route for a root that holds data files, appended only when
/// `duckdb` is on `PATH`. Short by design: the prompt is model-facing text,
/// and the block is the last thing in it so the prefix stays cacheable.
const FILE_SOURCES: &str = concat!(
    "This checkout holds data files. Analyse them with the `shell` tool: ",
    "`duckdb -readonly -json -c \"<SQL>\"`, always with a LIMIT; read files with ",
    "`read_csv_auto` or `read_parquet`; never write files or install extensions."
);

/// The variant when the root holds data files but `duckdb` is not on `PATH`:
/// the model must say so and give the install command, not guess.
const FILE_SOURCES_MISSING_DUCKDB: &str = concat!(
    "This checkout holds data files, but the `duckdb` CLI is not on PATH. ",
    "Say so and give the install command (`brew install duckdb`, or ",
    "https://duckdb.org/docs/installation/) instead of guessing at their contents."
);

/// What the root and host say about the shell route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileSources {
    /// No data file the bounded check reached: no guidance.
    NoData,
    /// A data file, and `duckdb` is on `PATH`: the shell route.
    DuckdbAvailable,
    /// A data file, but no `duckdb` on `PATH`: report it and give the install.
    DuckdbMissing,
}

/// Detects the guidance variant once. `path` is the caller's `PATH` value, so
/// tests pass their own and production reads it once.
pub(crate) fn detect(workspace: Option<&Workspace>, path: Option<&OsStr>) -> FileSources {
    if !holds_data_file(workspace) {
        return FileSources::NoData;
    }
    if duckdb_on_path(path) {
        FileSources::DuckdbAvailable
    } else {
        FileSources::DuckdbMissing
    }
}

/// The interactive session's gate: guidance only where the session advertises
/// the `shell` tool. Without the tool the guidance would name a route the
/// model cannot take, so a detected data file changes nothing.
pub(crate) fn for_session(shell_offered: bool, detected: FileSources) -> FileSources {
    if shell_offered {
        detected
    } else {
        FileSources::NoData
    }
}

/// The guidance block for `sources`, or `None` where none applies.
pub(crate) fn guidance(sources: FileSources) -> Option<&'static str> {
    match sources {
        FileSources::NoData => None,
        FileSources::DuckdbAvailable => Some(FILE_SOURCES),
        FileSources::DuckdbMissing => Some(FILE_SOURCES_MISSING_DUCKDB),
    }
}

/// Whether the bounded presence check reaches a data file. Only a definite
/// `found` counts: a walk a bound stopped answers "no answer", never "no
/// file", and guidance is offered only on a definite yes.
fn holds_data_file(workspace: Option<&Workspace>) -> bool {
    let Some(workspace) = workspace else {
        return false;
    };
    DATA_EXTENSIONS.iter().any(|extension| {
        workspace
            .holds_file_extension(extension, PresenceBounds::default())
            .found
    })
}

/// Whether a `duckdb` executable is on `path`. Metadata only: it never runs
/// the program and never reaches the network.
fn duckdb_on_path(path: Option<&OsStr>) -> bool {
    let Some(path) = path else {
        return false;
    };
    let name = if cfg!(windows) {
        "duckdb.exe"
    } else {
        "duckdb"
    };
    std::env::split_paths(path)
        .take(PATH_ENTRY_LIMIT)
        .any(|directory| directory.join(name).is_file())
}

#[cfg(test)]
#[path = "file_sources_tests.rs"]
mod file_sources_tests;
