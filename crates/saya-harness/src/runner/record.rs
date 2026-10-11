use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use saya_types::is_bare_name;

use super::{ProgramOutcome, RunnerError};

const MAX_RECORD_ATTEMPTS: usize = 16;
static NEXT_RECORD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// An atomic record-path collision differs from an error after a file was
/// created: only the former can safely retry with another unique candidate.
#[derive(Debug)]
enum CreateRecordError {
    Collision,
    Io(io::Error),
}

/// Persists the redacted outcome JSON under a new host-owned file name.
/// Existing entries, including symlinks, are never replaced or followed.
pub fn record_outcome(dir: &Path, outcome: &ProgramOutcome) -> Result<String, RunnerError> {
    if !is_bare_name(&outcome.program) {
        return Err(record_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            "program name is not a bare file name",
        )));
    }
    fs::create_dir_all(dir).map_err(record_error)?;
    let body = serde_json::to_vec_pretty(outcome).map_err(|error| {
        record_error(io::Error::new(
            io::ErrorKind::InvalidData,
            error.to_string(),
        ))
    })?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for _ in 0..MAX_RECORD_ATTEMPTS {
        let sequence = NEXT_RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(
            "{nanos}-{}-{sequence}-{}.json",
            std::process::id(),
            outcome.program
        ));
        match create_new_record(&path, &body) {
            Ok(()) => return Ok(path.display().to_string()),
            Err(CreateRecordError::Collision) => {}
            Err(CreateRecordError::Io(source)) => return Err(record_error(source)),
        }
    }
    Err(record_error(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique outcome record name",
    )))
}

fn create_new_record(path: &Path, body: &[u8]) -> Result<(), CreateRecordError> {
    let mut file = open_record_file(path).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            CreateRecordError::Collision
        } else {
            CreateRecordError::Io(source)
        }
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(CreateRecordError::Io)?;
    }
    // Once create_new succeeds, a partial write is an ambiguous outcome: keep
    // this unique file and return its error instead of retrying or replacing it.
    file.write_all(body).map_err(CreateRecordError::Io)
}

fn open_record_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

fn record_error(source: io::Error) -> RunnerError {
    RunnerError::RecordFailed { source }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::{
        collections::HashSet,
        fs,
        path::PathBuf,
        sync::{Arc, Barrier},
        thread,
    };

    use super::{
        CreateRecordError, ProgramOutcome, RunnerError, create_new_record, record_outcome,
    };
    use crate::runner::StreamCapture;

    fn directory(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("saya-runner-record-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn outcome(program: &str) -> ProgramOutcome {
        let stream = StreamCapture {
            text: String::new(),
            truncated: false,
            dropped_bytes: 0,
            total_bytes: 0,
        };
        ProgramOutcome {
            program: program.into(),
            pid: std::process::id(),
            exit_code: Some(0),
            signal: None,
            killed_by_timeout: false,
            cancelled: false,
            killed_orphans: false,
            duration_ms: 1,
            stdout: stream.clone(),
            stderr: stream,
        }
    }

    // Protects the actual atomic create-only primitive: a previously present
    // record remains byte-identical when a candidate path collides.
    #[test]
    fn existing_record_is_not_overwritten() {
        let root = directory("collision");
        let path = root.join("candidate.json");
        fs::write(&path, b"previous record").unwrap();

        assert!(matches!(
            create_new_record(&path, b"replacement"),
            Err(CreateRecordError::Collision)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"previous record");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_record_entry_is_not_followed() {
        let root = directory("symlink");
        let victim = root.join("victim.json");
        let entry = root.join("candidate.json");
        fs::write(&victim, b"outside record").unwrap();
        symlink(&victim, &entry).unwrap();

        assert!(matches!(
            create_new_record(&entry, b"replacement"),
            Err(CreateRecordError::Collision)
        ));
        assert_eq!(fs::read(&victim).unwrap(), b"outside record");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_outcomes_keep_distinct_complete_records() {
        const WRITERS: usize = 12;
        let root = directory("concurrent");
        let barrier = Arc::new(Barrier::new(WRITERS));
        let handles: Vec<_> = (0..WRITERS)
            .map(|_| {
                let root = root.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    record_outcome(&root, &outcome("pwd")).unwrap()
                })
            })
            .collect();
        let paths: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let unique: HashSet<_> = paths.iter().collect();
        let expected = serde_json::to_vec_pretty(&outcome("pwd")).unwrap();

        assert_eq!(unique.len(), WRITERS);
        assert_eq!(fs::read_dir(&root).unwrap().count(), WRITERS);
        for path in &paths {
            assert_eq!(fs::read(PathBuf::from(path.as_str())).unwrap(), expected);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_shaped_program_is_refused_before_record_creation() {
        let root = std::env::temp_dir().join(format!(
            "saya-runner-record-program-path-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);

        assert!(matches!(
            record_outcome(&root, &outcome("../escape")),
            Err(RunnerError::RecordFailed { .. })
        ));
        assert!(!root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn outcome_record_permissions_are_private() {
        let root = directory("mode");
        let path = record_outcome(&root, &outcome("pwd")).unwrap();

        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
