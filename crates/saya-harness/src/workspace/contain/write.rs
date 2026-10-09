use super::*;

impl Workspace {
    /// Writes a workspace file atomically: temp file at 0600, fsync, rename
    /// over the target. The rename replaces whatever the path names without
    /// following it, so content cannot land outside the root; the path is
    /// re-verified afterwards so a post-write swap is reported rather than
    /// pretended away. No execute bits are ever set.
    pub fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        if bytes.len() > MAX_IO_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: bytes.len() as u64,
                max: MAX_IO_BYTES as u64,
            });
        }
        #[cfg(unix)]
        return super::super::anchored::write(self, rel, bytes);

        #[cfg(not(unix))]
        {
            let path = self.target(rel, true)?;
            if let Ok(meta) = fs::symlink_metadata(&path) {
                if meta.file_type().is_symlink() {
                    return Err(HarnessError::SymlinkRefused {
                        path: rel.to_string(),
                    });
                }
                if !meta.is_file() {
                    return Err(HarnessError::NotRegularFile {
                        path: rel.to_string(),
                    });
                }
            }
            let parent = path.parent().unwrap_or(self.root.as_path()).to_path_buf();
            let (temp_path, mut temp) = self.create_temp(&parent, rel)?;
            temp.write_all(bytes)
                .map_err(|error| io_error("write workspace temp", &temp_path, error))?;
            temp.sync_all()
                .map_err(|error| io_error("sync workspace temp", &temp_path, error))?;
            let written = file_identity(&temp)
                .map_err(|error| io_error("stat workspace temp", &temp_path, error))?;
            drop(temp);
            replace_file(&temp_path, &path)?;
            let final_meta = fs::symlink_metadata(&path)
                .map_err(|error| io_error("verify written workspace file", &path, error))?;
            let exec_bits = false;
            let committed = path_identity(&path)
                .map_err(|error| io_error("verify written workspace file", &path, error))?;
            if committed != written || !final_meta.is_file() || exec_bits {
                return Err(HarnessError::IdentityChanged {
                    path: rel.to_string(),
                });
            }
            Ok(())
        }
    }
    /// Opens a fresh temp file at 0600 inside the validated parent with
    /// `O_CREAT|O_EXCL`, so a pre-planted name (even a link) can never be
    /// adopted. The non-unix shape; the unix write anchors the temp to the
    /// walked parent descriptor instead. Shared with the range patch's
    /// non-unix commit path.
    #[cfg(not(unix))]
    pub(crate) fn create_temp(
        &self,
        parent: &Path,
        rel: &str,
    ) -> Result<(PathBuf, fs::File), HarnessError> {
        for attempt in 0..8 {
            let name = format!(".saya-tmp-{}-{}-{attempt}", process::id(), nanos());
            let candidate = parent.join(name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            match options.open(&candidate) {
                Ok(file) => return Ok((candidate, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("create workspace temp", &candidate, error)),
            }
        }
        Err(HarnessError::Io {
            context: format!("name a unique workspace temp file for {rel}"),
            source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, "temp name collisions"),
        })
    }
}
