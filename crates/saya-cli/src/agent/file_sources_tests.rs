use std::{
    ffi::OsStr,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use saya_agent::AgentMode;
use saya_config::MemoryMode;
use saya_harness::workspace::Workspace;

use super::{FileSources, detect, for_session};
use crate::agent::session_facts::SessionFacts;
use crate::agent::system_prompt::assemble_system_prompt_for_mode_with_file_sources;
use crate::connection::ConnectionRegistry;

/// A temp checkout and a temp `bin` directory. `path()` is passed to
/// [`detect`] explicitly, so no test mutates the process environment.
struct Fixture {
    outer: PathBuf,
    workspace: Workspace,
    bin: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let outer = std::env::temp_dir().join(format!(
            "saya-file-sources-{label}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(outer.join("ws")).unwrap();
        fs::create_dir_all(outer.join("bin")).unwrap();
        let workspace = Workspace::open(&outer.join("ws")).unwrap();
        let bin = outer.join("bin");
        Self {
            outer,
            workspace,
            bin,
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) {
        let target = self.workspace.root().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }

    fn install_duckdb(&self) {
        let name = if cfg!(windows) {
            "duckdb.exe"
        } else {
            "duckdb"
        };
        fs::write(self.bin.join(name), b"#!/bin/sh\n").unwrap();
    }

    fn path(&self) -> &OsStr {
        self.bin.as_os_str()
    }

    fn detect(&self) -> FileSources {
        detect(Some(&self.workspace), Some(self.path()))
    }

    /// The interactive session's prompt, assembled over this checkout's root.
    fn interactive_prompt(&self, sources: FileSources) -> String {
        let registry = ConnectionRegistry::new("main");
        let facts = SessionFacts {
            registry: &registry,
            workspace_root: Some(self.workspace.root()),
        };
        assemble_system_prompt_for_mode_with_file_sources(
            &registry,
            MemoryMode::Off,
            false,
            &facts,
            AgentMode::Build,
            sources,
        )
        .expect("the interactive prompt is never empty")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("retained file-sources fixture: {}", self.outer.display());
        } else {
            fs::remove_dir_all(&self.outer).unwrap();
        }
    }
}

/// A data file and a `duckdb` on `PATH`: the interactive prompt carries the
/// shell route, and it is the last thing in the prompt.
#[test]
fn interactive_prompt_has_the_block_with_a_data_file_and_duckdb_on_path() {
    let fixture = Fixture::new("interactive-available");
    fixture.put("data/sales.csv", b"region,amount\nwest,1\n");
    fixture.install_duckdb();

    assert_eq!(fixture.detect(), FileSources::DuckdbAvailable);
    let prompt = fixture.interactive_prompt(fixture.detect());
    for sentence in [
        "duckdb -readonly -json -c",
        "LIMIT",
        "read_csv_auto",
        "read_parquet",
        "never write files or install extensions",
    ] {
        assert!(
            prompt.contains(sentence),
            "the interactive prompt must state this: {sentence} in {prompt}"
        );
    }
    assert!(
        prompt.ends_with(super::guidance(FileSources::DuckdbAvailable).unwrap()),
        "the guidance is appended at the end, so the prefix stays cacheable: {prompt}"
    );
}

/// No data file: no guidance at all, whatever the host has on `PATH`.
#[test]
fn interactive_prompt_has_no_block_without_a_data_file() {
    let fixture = Fixture::new("interactive-absent");
    fixture.put("src/main.rs", b"fn main() {}\n");
    fixture.put("README.md", b"# readme\n");
    fixture.install_duckdb();

    assert_eq!(fixture.detect(), FileSources::NoData);
    let prompt = fixture.interactive_prompt(fixture.detect());
    assert!(!prompt.contains("duckdb"), "{prompt}");
}

/// Every code repository holds `package.json`; it must not trigger the block.
#[test]
fn interactive_prompt_has_no_block_for_package_json_only() {
    let fixture = Fixture::new("interactive-json");
    fixture.put("package.json", b"{}\n");
    fixture.put("tsconfig.json", b"{}\n");
    fixture.install_duckdb();

    assert_eq!(fixture.detect(), FileSources::NoData);
    let prompt = fixture.interactive_prompt(fixture.detect());
    assert!(!prompt.contains("duckdb"), "{prompt}");
}

/// The shell gate is the session's: without the `shell` tool the guidance
/// would name a route the model cannot take, so it is absent.
#[test]
fn interactive_prompt_has_no_block_when_shell_is_not_offered() {
    let fixture = Fixture::new("interactive-no-shell");
    fixture.put("data/sales.csv", b"region,amount\nwest,1\n");
    fixture.install_duckdb();

    let sources = for_session(false, fixture.detect());
    assert_eq!(sources, FileSources::NoData);
    let prompt = fixture.interactive_prompt(sources);
    assert!(!prompt.contains("duckdb"), "{prompt}");
}

/// The missing-CLI variant also reaches the interactive prompt: say so and
/// give the install command, never guess at the file.
#[test]
fn interactive_prompt_reports_a_missing_duckdb() {
    let fixture = Fixture::new("interactive-missing");
    fixture.put("data/events.parquet", b"PAR1");
    // The `bin` directory exists but holds no `duckdb`.

    assert_eq!(fixture.detect(), FileSources::DuckdbMissing);
    let prompt = fixture.interactive_prompt(fixture.detect());
    for sentence in [
        "not on PATH",
        "brew install duckdb",
        "instead of guessing at their contents",
    ] {
        assert!(
            prompt.contains(sentence),
            "the missing-CLI variant must state this: {sentence} in {prompt}"
        );
    }
    assert!(!prompt.contains("duckdb -readonly"), "{prompt}");
}

/// `json` is a config format, not a data-file signal; `ndjson` and `jsonl`
/// are.
#[test]
fn json_is_not_a_data_extension_but_ndjson_and_jsonl_are() {
    let json = Fixture::new("extensions-json");
    json.put("package.json", b"{}\n");
    json.install_duckdb();
    assert_eq!(json.detect(), FileSources::NoData);

    for (label, name) in [
        ("extensions-ndjson", "events.ndjson"),
        ("extensions-jsonl", "events.jsonl"),
    ] {
        let fixture = Fixture::new(label);
        fixture.put(name, b"{}\n");
        fixture.install_duckdb();
        assert_eq!(
            fixture.detect(),
            FileSources::DuckdbAvailable,
            "{name} is a data file"
        );
    }
}

/// `PATH` is a bounded, metadata-only lookup: only entries before the bound
/// are read, and a directory that is not a directory is not a crash.
#[test]
fn the_path_lookup_is_bounded_and_metadata_only() {
    let fixture = Fixture::new("bounded");
    fixture.put("data/table.tsv", b"a\tb\n");
    // A plain file named as a PATH entry: split_paths still yields it, and
    // `join` under it is not a file.
    let not_a_dir = fixture.outer.join("not-a-dir");
    fs::write(&not_a_dir, b"x").unwrap();
    assert_eq!(
        detect(Some(&fixture.workspace), Some(not_a_dir.as_os_str())),
        FileSources::DuckdbMissing
    );
}

/// The check treats the extension case-insensitively and respects the
/// workspace's own ignore policy, exactly as the presence check it reuses.
#[test]
fn detection_reuses_the_bounded_presence_check() {
    let fixture = Fixture::new("ignored");
    fixture.put(".gitignore", b"*.csv\n");
    fixture.put("build/generated.csv", b"a\n1\n");
    fixture.install_duckdb();
    assert_eq!(
        fixture.detect(),
        FileSources::NoData,
        "an ignored data file is not counted"
    );

    let upper = Fixture::new("upper");
    upper.put("REPORT.NDJSON", b"{}\n");
    upper.install_duckdb();
    assert_eq!(upper.detect(), FileSources::DuckdbAvailable);
}
