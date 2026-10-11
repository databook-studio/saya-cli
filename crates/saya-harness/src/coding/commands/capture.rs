//! Complete immutable writer snapshots, independent of mutable output files.
use super::output::{CommandOutputError, CommandOutputStore, OUTPUT_STORE_CAP_BYTES, OutputStream};
use std::{fmt, sync::Arc};

/// Actual redacted bytes accepted by this stream's writer. No parser, process
/// success, approval or receipt is implied. Capture before consuming `join`.
/// Each snapshot is bounded; callers must also bound retained collections.
#[derive(Clone, PartialEq, Eq)]
pub struct CommandOutputCapture {
    stream: OutputStream,
    directory_id: String,
    bytes: Arc<[u8]>,
}
impl CommandOutputCapture {
    pub fn stream(&self) -> OutputStream {
        self.stream
    }
    /// Historical opaque store leaf, never current-file provenance or a path grant.
    pub fn directory_id(&self) -> &str {
        &self.directory_id
    }
    /// Complete raw redacted bytes; validate UTF8 once in the selected parser.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl fmt::Debug for CommandOutputCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandOutputCapture")
            .field("stream", &self.stream)
            .field("directory_id", &self.directory_id)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
impl CommandOutputStore {
    pub(super) fn capture_complete(
        &self,
        stream: OutputStream,
    ) -> Result<CommandOutputCapture, CommandOutputError> {
        let status = self
            .output_status()
            .ok_or(CommandOutputError::OutputNotComplete)?;
        if status.capped || status.dropped_bytes != 0 || status.error.is_some() {
            return Err(CommandOutputError::OutputNotComplete);
        }
        let settled = self
            .sink(stream)
            .result()
            .ok_or(CommandOutputError::OutputNotComplete)?;
        let bytes = settled
            .captured_bytes
            .ok_or(CommandOutputError::OutputNotComplete)?;
        if bytes.len() > OUTPUT_STORE_CAP_BYTES || bytes.len() as u64 != settled.stored_bytes {
            return Err(CommandOutputError::OutputNotComplete);
        }
        let directory_id = self
            .dir()
            .file_name()
            .and_then(|leaf| leaf.to_str())
            .ok_or(CommandOutputError::OutputNotComplete)?
            .to_owned();
        Ok(CommandOutputCapture {
            stream,
            directory_id,
            bytes,
        })
    }
}
