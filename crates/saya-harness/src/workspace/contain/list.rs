use super::*;

impl Workspace {
    /// Lists a workspace directory, bounded by `max_entries`. The empty
    /// argument, `.`, or `./` names the workspace root itself. A directory
    /// holding more than `max_entries` entries is served short — the first
    /// `max_entries` by name, with `truncated` set and the rest counted in
    /// `omitted` — never refused: the model learns the directory is larger
    /// instead of losing the call and narrowing by guess.
    pub fn list(&self, rel: &str, max_entries: usize) -> Result<ListOutcome, HarnessError> {
        // An absolute path under the root is its relative spelling; the
        // absolute root itself becomes the empty path the walker reads as
        // the root. Search already treats `.` as the workspace root, and
        // models send that same path on `list`; the component walker would
        // otherwise refuse it as InvalidPath before any directory is opened.
        let rel = relative_argument(&self.root, rel)?;
        let rel = rel.as_ref();
        let rel = if matches!(rel, "." | "./") { "" } else { rel };
        if max_entries > MAX_LIST_ENTRIES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: max_entries as u64,
                max: MAX_LIST_ENTRIES as u64,
            });
        }
        #[cfg(unix)]
        return super::super::anchored::list(self, rel, max_entries);

        #[cfg(not(unix))]
        {
            let dir = if rel.is_empty() {
                self.root.clone()
            } else {
                let path = self.target(rel, false)?;
                let meta = fs::symlink_metadata(&path)
                    .map_err(|error| io_error("list workspace directory", &path, error))?;
                if meta.file_type().is_symlink() {
                    return Err(HarnessError::SymlinkRefused {
                        path: rel.to_string(),
                    });
                }
                if !meta.is_dir() {
                    return Err(HarnessError::NotRegularFile {
                        path: rel.to_string(),
                    });
                }
                path
            };
            let read = fs::read_dir(&dir)
                .map_err(|error| io_error("list workspace directory", &dir, error))?;
            let mut entries = Vec::new();
            for entry in read {
                let entry =
                    entry.map_err(|error| io_error("list workspace directory", &dir, error))?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| HarnessError::Io {
                        context: format!("workspace entry name is not UTF-8 in {}", dir.display()),
                        source: std::io::Error::new(std::io::ErrorKind::InvalidData, "entry name"),
                    })?;
                let file_type = entry
                    .file_type()
                    .map_err(|error| io_error("list workspace directory", &dir, error))?;
                let kind = if file_type.is_symlink() {
                    EntryKind::Symlink
                } else if file_type.is_dir() {
                    EntryKind::Dir
                } else if file_type.is_file() {
                    EntryKind::File
                } else {
                    EntryKind::Other
                };
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map(|meta| meta.len()).unwrap_or(0);
                let mtime = meta.as_ref().and_then(|meta| meta.modified().ok());
                entries.push(ListEntry {
                    name,
                    kind,
                    size,
                    mtime,
                });
                // The enumeration itself is bounded: one entry past the ceiling
                // is enough to know the directory holds more than was counted,
                // so a flood costs a bounded read rather than an unbounded one.
                if entries.len() > enumeration_cap(max_entries) {
                    break;
                }
            }
            Ok(serve_bounded(entries, max_entries))
        }
    }
}

/// How many names the enumeration may read before it stops: the caller's own
/// bound, floored at [`MAX_LIST_ENTRIES`] so a caller asking for few entries
/// still learns the exact size of a moderately large directory. Callers read
/// one name past it, so "more than this" is distinguishable from "exactly
/// this".
pub(crate) fn enumeration_cap(max_entries: usize) -> usize {
    max_entries.max(MAX_LIST_ENTRIES)
}

/// Serves at most `max_entries` of the enumerated `entries` and reports what
/// is left out. `entries` holds at most [`enumeration_cap`] + 1 names, so
/// entries past it are counted only as a lower bound: a quietly shortened
/// listing must never read as the whole directory, and a listing must never
/// refuse a directory that is merely large.
pub(crate) fn serve_bounded(mut entries: Vec<ListEntry>, max_entries: usize) -> ListOutcome {
    let seen = entries.len();
    let ceiling = seen > enumeration_cap(max_entries);
    if ceiling {
        entries.truncate(enumeration_cap(max_entries));
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let served = entries.len().min(max_entries);
    let omitted = (seen - served) as u64;
    if omitted == 0 && !ceiling {
        return ListOutcome {
            entries,
            truncated: false,
            omitted: 0,
            omitted_exact: true,
        };
    }
    entries.truncate(served);
    ListOutcome {
        entries,
        truncated: true,
        omitted,
        omitted_exact: !ceiling,
    }
}
