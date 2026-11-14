use super::*;

impl Workspace {
    /// Whether the path currently carries an executable bit. This is one
    /// no-follow anchored stat: an absent path is `false`, and a symlink,
    /// directory, or other non-file refuses rather than guessing. Executable
    /// bits are a unix concept; elsewhere the answer is always `false`.
    pub fn is_executable(&self, rel: &str) -> Result<bool, HarnessError> {
        #[cfg(unix)]
        {
            let anchor = self.anchor(rel, false)?;
            match anchor.stat() {
                Some(stat) if stat.is_file() => Ok(stat.has_exec_bits()),
                Some(_) => Err(HarnessError::NotRegularFile {
                    path: rel.to_string(),
                }),
                None => Ok(false),
            }
        }
        #[cfg(not(unix))]
        {
            let path = self.target(rel, false)?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    Err(HarnessError::SymlinkRefused {
                        path: rel.to_string(),
                    })
                }
                Ok(metadata) if metadata.is_file() => Ok(false),
                Ok(_) => Err(HarnessError::NotRegularFile {
                    path: rel.to_string(),
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(io_error("stat workspace file", &path, error)),
            }
        }
    }
}
