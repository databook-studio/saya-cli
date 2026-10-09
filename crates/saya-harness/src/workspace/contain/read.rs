use super::*;

impl Workspace {
    /// Reads a workspace file under containment: validation, a no-follow
    /// open, and a post-open identity check so the bytes served belong to
    /// the file that was scanned. At most `max_bytes` are returned, with
    /// `truncated` set when the file holds more; the digest covers the whole
    /// file — hashed in the same open, streamed in chunks — so a truncated
    /// read still names the state an edit precondition can state. Files
    /// larger than the I/O backstop refuse before any byte is hashed; the
    /// refusal names the scanned size and the bound.
    pub fn read(&self, rel: &str, max_bytes: u64) -> Result<ReadFile, HarnessError> {
        self.read_with_limit(rel, max_bytes, MAX_IO_BYTES as u64)
    }
    /// Reads one complete file for scratch CSV import through the same
    /// contained open as `read`, with only this operation's 32 MiB ceiling.
    pub fn read_for_scratch_import(&self, rel: &str) -> Result<ReadFile, HarnessError> {
        self.read_with_limit(
            rel,
            MAX_SCRATCH_IMPORT_BYTES as u64,
            MAX_SCRATCH_IMPORT_BYTES as u64,
        )
    }
    fn read_with_limit(
        &self,
        rel: &str,
        max_bytes: u64,
        file_limit: u64,
    ) -> Result<ReadFile, HarnessError> {
        if max_bytes > file_limit {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: max_bytes,
                max: file_limit,
            });
        }
        #[cfg(unix)]
        return super::super::anchored::read(self, rel, max_bytes, file_limit);

        #[cfg(not(unix))]
        {
            use sha2::{Digest, Sha256};

            let path = self.target(rel, false)?;
            let pre = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(HarnessError::NotFound {
                        path: rel.to_string(),
                    });
                }
                Err(error) => return Err(io_error("read workspace file", &path, error)),
            };
            if pre.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if !pre.is_file() {
                return Err(HarnessError::NotRegularFile {
                    path: rel.to_string(),
                });
            }
            if pre.len() > file_limit {
                return Err(HarnessError::BoundsExceeded {
                    path: rel.to_string(),
                    found: pre.len(),
                    max: file_limit,
                });
            }
            let file = self.open_verified(&path, &pre, rel)?;
            let mut bytes = Vec::new();
            let mut hasher = Sha256::new();
            let mut chunk = [0u8; 64 * 1024];
            let mut kept = 0u64;
            let mut scanned = 0u64;
            loop {
                use std::io::Read as _;

                let read = (&file)
                    .read(&mut chunk)
                    .map_err(|error| io_error("read workspace file", &path, error))?;
                if read == 0 {
                    break;
                }
                scanned += read as u64;
                if scanned > file_limit {
                    return Err(HarnessError::BoundsExceeded {
                        path: rel.to_string(),
                        found: scanned,
                        max: file_limit,
                    });
                }
                hasher.update(&chunk[..read]);
                let room = max_bytes.saturating_sub(kept) as usize;
                let take = read.min(room);
                bytes.extend_from_slice(&chunk[..take]);
                kept += take as u64;
            }
            let digest: String = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            Ok(ReadFile {
                bytes,
                size: pre.len(),
                truncated: pre.len() > max_bytes,
                digest,
            })
        }
    }
    /// Opens `path` for reading after the pre-open scan `pre`: the open
    /// itself is no-follow (a link swapped into the final component fails
    /// the open), and the opened file's (dev, inode) must match the scan.
    pub(crate) fn open_verified(
        &self,
        path: &Path,
        pre: &fs::Metadata,
        rel: &str,
    ) -> Result<fs::File, HarnessError> {
        #[cfg(windows)]
        let _ = pre;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let file = options.open(path).map_err(|error| {
            #[cfg(unix)]
            if error.raw_os_error() == Some(libc::ELOOP) {
                return HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                };
            }
            io_error("open workspace file", path, error)
        })?;
        let opened = file
            .metadata()
            .map_err(|error| io_error("stat opened workspace file", path, error))?;
        #[cfg(windows)]
        if super::windows_identity::is_reparse_point(&file)
            .map_err(|error| io_error("stat opened workspace file", path, error))?
        {
            return Err(HarnessError::SymlinkRefused {
                path: rel.to_string(),
            });
        }
        if !opened.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: rel.to_string(),
            });
        }
        #[cfg(windows)]
        {
            let current = path_identity(path)
                .map_err(|error| io_error("verify opened workspace file", path, error))?;
            let opened = file_identity(&file)
                .map_err(|error| io_error("stat opened workspace file", path, error))?;
            if opened != current {
                return Err(HarnessError::IdentityChanged {
                    path: rel.to_string(),
                });
            }
        }
        #[cfg(not(windows))]
        if identity_of(&opened) != identity_of(pre) {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(file)
    }
}
