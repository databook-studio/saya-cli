use super::*;

impl Workspace {
    /// Reads and hashes one contained open, checking cancellation every 64 KiB.
    pub fn read_controlled(
        &self,
        rel: &str,
        max_bytes: u64,
        cancellation: &saya_agent::CancellationToken,
    ) -> Result<ReadFile, ControlledReadError> {
        if max_bytes > MAX_IO_BYTES as u64 {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: max_bytes,
                max: MAX_IO_BYTES as u64,
            }
            .into());
        }
        if cancellation.is_cancelled() {
            return Err(ControlledReadError::Cancelled);
        }
        #[cfg(unix)]
        return super::super::super::anchored::read_controlled(
            self,
            rel,
            max_bytes,
            MAX_IO_BYTES as u64,
            cancellation,
        );

        #[cfg(not(unix))]
        self.read_with_limit_controlled(rel, max_bytes, MAX_IO_BYTES as u64, cancellation)
    }

    #[cfg(not(unix))]
    fn read_with_limit_controlled(
        &self,
        rel: &str,
        max_bytes: u64,
        file_limit: u64,
        cancellation: &saya_agent::CancellationToken,
    ) -> Result<ReadFile, ControlledReadError> {
        use sha2::{Digest, Sha256};
        use std::io::Read as _;

        if cancellation.is_cancelled() {
            return Err(ControlledReadError::Cancelled);
        }
        let path = self.target(rel, false)?;
        let pre = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(HarnessError::NotFound {
                    path: rel.to_string(),
                }
                .into());
            }
            Err(error) => return Err(io_error("read workspace file", &path, error).into()),
        };
        if pre.file_type().is_symlink() {
            return Err(HarnessError::SymlinkRefused {
                path: rel.to_string(),
            }
            .into());
        }
        if !pre.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: rel.to_string(),
            }
            .into());
        }
        if pre.len() > file_limit {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: pre.len(),
                max: file_limit,
            }
            .into());
        }
        let file = self.open_verified(&path, &pre, rel)?;
        let mut bytes = Vec::new();
        let mut hasher = Sha256::new();
        let mut chunk = [0u8; 64 * 1024];
        let mut kept = 0u64;
        let mut scanned = 0u64;
        loop {
            if cancellation.is_cancelled() {
                return Err(ControlledReadError::Cancelled);
            }
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
                }
                .into());
            }
            hasher.update(&chunk[..read]);
            let take = read.min(max_bytes.saturating_sub(kept) as usize);
            bytes.extend_from_slice(&chunk[..take]);
            kept += take as u64;
        }
        if cancellation.is_cancelled() {
            return Err(ControlledReadError::Cancelled);
        }
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(ReadFile {
            bytes,
            size: scanned,
            truncated: scanned > max_bytes,
            digest,
        })
    }
}
