use super::cell::Cell;
use super::*;

// -- reading the lanes the capture reaches --------------------------------

/// Every `tool`-role message the provider received, joined — the
/// model-facing copy of the tool result.
pub(super) fn tool_lane(requests: &[ChatRequest]) -> String {
    requests
        .iter()
        .flat_map(|request| request.messages.iter())
        .filter(|message| message.role == "tool")
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every regular file under `root`, at any depth.
pub(super) fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

/// Byte-scans every file for the needle; any hit fails with the file
/// named. The scan is exact-window, so a near-miss is not a hit.
pub(super) fn assert_absent(files: &[PathBuf], needle: &str, lane: &str) {
    for file in files {
        let bytes = fs::read(file).unwrap_or_default();
        assert!(
            !bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes()),
            "{lane}: the planted credential reached {}: {file:?}",
            file.display()
        );
    }
}

/// Byte-scans raw bytes (the store database and its sidecars, the tool
/// lane text) for the needle.
pub(super) fn assert_absent_bytes(bytes: &[u8], needle: &str, lane: &str) {
    assert!(
        !bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes()),
        "{lane}: the planted credential appears in the bytes"
    );
}

/// The store database's raw bytes plus any populated `-wal`/`-shm`
/// sidecars — a freshly written row may live only in the WAL, so
/// scanning all three is what makes the check honest.
pub(super) fn store_bytes(cell: &Cell) -> Vec<u8> {
    let db = cell.root.join("state.sqlite3");
    let mut out =
        fs::read(&db).unwrap_or_else(|error| panic!("the store database must exist: {error}"));
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", db.display()));
        if sidecar.exists() {
            out.extend(fs::read(&sidecar).unwrap_or_default());
        }
    }
    out
}

/// The runner's disk records: the files under the workspace's
/// `run_program/` directory.
pub(super) fn record_files(cell: &Cell) -> Vec<PathBuf> {
    files_under(cell.run.root())
        .into_iter()
        .filter(|path| {
            path.components()
                .any(|component| component.as_os_str() == "run_program")
        })
        .collect()
}

/// Every place the planted value is forbidden, swept: the whole run
/// directory (generated configs, journal, disk records, workspace), the
/// store database with its sidecars, and the journal's parsed events.
pub(super) fn sweep_run(cell: &Cell, value: &str) {
    assert_absent(&files_under(cell.run.root()), value, "the run directory");
    let store = store_bytes(cell);
    assert_absent_bytes(&store, value, "the store database");
    let journal = fs::read_to_string(cell.run.root().join(EVENTS_FILE))
        .expect("the journal must be readable");
    assert!(
        !journal.contains(value),
        "the journal carries credential material: {journal}"
    );
    let events = Journal::open(cell.run.root())
        .read()
        .expect("the journal parses");
    assert!(
        !format!("{events:?}").contains(value),
        "the journal's parsed events carry credential material"
    );
}

pub(super) fn cleanup(cell: &Cell) {
    let _ = fs::remove_dir_all(&cell.root);
}
