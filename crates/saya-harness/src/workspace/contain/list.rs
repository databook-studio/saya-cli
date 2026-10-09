use super::*;

impl Workspace {
    /// Lists a workspace directory, bounded by `max_entries`. The empty
    /// argument names the workspace root itself.
    pub fn list(&self, rel: &str, max_entries: usize) -> Result<Vec<ListEntry>, HarnessError> {
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
                let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                entries.push(ListEntry { name, kind, size });
                // Checked after the push, so the refusal fires when a directory
                // holds MORE than `max_entries` entries — the (max+1)-th entry
                // trips the bound with found > max, matching the search bounds
                // and the walk's visited bound; a quietly shortened listing would
                // read as the whole directory.
                if entries.len() > max_entries {
                    return Err(HarnessError::BoundsExceeded {
                        path: rel.to_string(),
                        found: entries.len() as u64,
                        max: max_entries as u64,
                    });
                }
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(entries)
        }
    }
}
