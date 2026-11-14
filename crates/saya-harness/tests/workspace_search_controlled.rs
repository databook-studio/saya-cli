//! Public scoped search contracts; real Git is an independent ignore oracle.
use saya_agent::CancellationToken;
use saya_harness::workspace::{
    GrepOptions, MIN_RESULT_BYTES, SearchEntry, SearchKind, SearchLimits, SearchMatch,
    SearchOutcome, SearchQuery, Workspace,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

mod workspace_search_controlled {
    pub(super) mod bounds;
    pub(super) mod modes;
    pub(super) mod modes_bounds;
    pub(super) mod order;
    pub(super) mod policy;
    pub(super) mod semantics;
}
struct Fixture {
    outer: PathBuf,
    workspace: Workspace,
}
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let outer = std::env::temp_dir().join(format!(
            "saya-search-controlled-{label}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&outer).unwrap();
        fs::create_dir(outer.join("ws")).unwrap();
        let workspace = Workspace::open(&outer.join("ws")).unwrap();
        Self { outer, workspace }
    }
    fn put(&self, path: &str, bytes: &[u8]) {
        let target = self.workspace.root().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
    /// Writes one file and stamps its modification time: the ordering's only
    /// input, and the one thing `put` cannot control.
    fn put_at(&self, path: &str, bytes: &[u8], modified: SystemTime) {
        self.put(path, bytes);
        fs::File::options()
            .write(true)
            .open(self.workspace.root().join(path))
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }
    fn search(&self, query: &SearchQuery, limits: SearchLimits) -> SearchOutcome {
        self.workspace
            .search_controlled(query, &limits, &CancellationToken::new())
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("retained search fixture: {}", self.outer.display());
        } else {
            fs::remove_dir_all(&self.outer).unwrap();
        }
    }
}
fn query(kind: SearchKind, pattern: &str, path: &str) -> SearchQuery {
    SearchQuery {
        kind,
        pattern: pattern.into(),
        path: path.into(),
        include: None,
        regex: false,
        case_insensitive: false,
        entry: None,
        grep: GrepOptions::default(),
        private_prefixes: Vec::new(),
    }
}
/// The clock the ordering reads, in whole seconds since the epoch.
fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}
/// A glob query filtered to one entry kind.
fn typed(pattern: &str, entry: SearchEntry) -> SearchQuery {
    SearchQuery {
        entry: Some(entry),
        ..query(SearchKind::Glob, pattern, "")
    }
}
/// The served paths in name order, for the assertions that are about *which*
/// paths a filter admits rather than the mtime order they arrive in.
fn served(outcome: &SearchOutcome) -> Vec<&str> {
    let mut paths = paths(outcome);
    paths.sort_unstable();
    paths
}
fn paths(outcome: &SearchOutcome) -> Vec<&str> {
    outcome
        .matches
        .iter()
        .map(|hit| match hit {
            SearchMatch::Path(path) | SearchMatch::Count { path, .. } => path.as_str(),
            SearchMatch::Line { path, .. } => path.as_str(),
        })
        .collect()
}

async fn git_ignored(fixture: &Fixture, candidates: &[&str]) -> std::collections::BTreeSet<String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new("git");
    command
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.ignoreCase=false",
            "-c",
            "core.excludesFile=/dev/null",
            "init",
            "--quiet",
        ])
        .current_dir(fixture.workspace.root())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0")
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "independent Git oracle setup failed"
    );
    let mut child = tokio::process::Command::new("git")
        .args([
            "-c",
            "core.ignoreCase=false",
            "-c",
            "core.excludesFile=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "check-ignore",
            "--no-index",
            "--verbose",
            "--non-matching",
            "-z",
            "--stdin",
        ])
        .current_dir(fixture.workspace.root())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let bytes = candidates
        .iter()
        .flat_map(|path| path.bytes().chain(std::iter::once(0)))
        .collect::<Vec<_>>();
    child.stdin.take().unwrap().write_all(&bytes).await.unwrap();
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "Git oracle protocol failed"
    );
    // Preserve empty non-match fields: each candidate has exactly four NUL fields.
    let records: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
    assert_eq!(
        records.len(),
        candidates.len() * 4 + 1,
        "Git oracle record shape"
    );
    records[..records.len() - 1]
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|record| !record[2].is_empty() && !record[2].starts_with(b"!"))
        .map(|record| String::from_utf8(record[3].to_vec()).unwrap())
        .collect()
}
