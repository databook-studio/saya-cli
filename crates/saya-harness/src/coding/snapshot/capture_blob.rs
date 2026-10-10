//! Captures one planned HEAD-blob entry: the entry's committed bytes are
//! read through the typed Git process owner, checked against the plan's
//! declared size and the remaining aggregate budget before the Git call,
//! verified, hashed, and streamed into the caller's sink in bounded 64 KiB
//! chunks. This is the GitBlob counterpart of the working-tree reader; the
//! orchestrator composes it per entry and owns no staging here.
//!
//! On ANY error the sink may already hold partial bytes: the caller must
//! discard the sink, the same contract as the working-tree reader.

#[cfg(not(unix))]
mod non_unix_blob {
    use std::io::Write;

    use saya_agent::CancellationToken;

    use crate::coding::git::{GitRepository, GitService};

    use super::super::{
        capture_reader::CapturedFile,
        manifest::{RedactedIoError, SnapshotError},
        types::SnapshotFileEntry,
    };

    pub async fn capture_head_blob(
        service: &GitService,
        repository: &GitRepository,
        entry: &SnapshotFileEntry,
        remaining_aggregate_bytes: u64,
        sink: &mut dyn Write,
        cancellation: &CancellationToken,
    ) -> Result<CapturedFile, SnapshotError> {
        let _ = (
            service,
            repository,
            entry,
            remaining_aggregate_bytes,
            sink,
            cancellation,
        );
        Err(SnapshotError::CaptureIo {
            description: "snapshot capture requires a unix host".to_owned(),
            source: Some(RedactedIoError::new(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "non-unix host",
            ))),
        })
    }
}

#[cfg(not(unix))]
pub use non_unix_blob::capture_head_blob;

#[cfg(unix)]
mod unix_blob {
    use std::io::Write;

    use saya_agent::CancellationToken;
    use sha2::{Digest, Sha256};
    use tokio::time::{Duration, Instant};

    use crate::coding::git::{GitObjectFormat, GitRepository, GitService};

    use super::super::{
        capture_reader::{CapturedFile, reader_error::capture_io},
        manifest::{SnapshotError, digest},
        types::{MAX_SNAPSHOT_FILE_BYTES, SnapshotContentSource, SnapshotFileEntry},
    };

    const CHUNK_BYTES: usize = 64 * 1024;
    const SNAPSHOT_BLOB_DEADLINE: Duration = Duration::from_secs(30);

    /// Captures one planned HEAD-blob entry into `sink`. The entry must be a
    /// GitBlob source; the declared size must fit the per-file ceiling and
    /// `remaining_aggregate_bytes`, both enforced BEFORE the Git call. The
    /// blob is read through the service's guarded process route and must
    /// arrive at exactly the declared size, else `BlobSizeMismatch`.
    ///
    /// Digest contract: the captured SHA-256 is the hash of the received
    /// bytes. When the repository object format is SHA-256, the blob is
    /// additionally verified against its object id
    /// (sha256("blob <len>\0" + bytes)); a mismatch is `BlobDigestMismatch`,
    /// a corrupt object store. Residual for SHA-1 repositories: no
    /// blob-vs-object-id verification is performed — adding a sha1
    /// dependency needs lead approval at the final gate, so Git's object
    /// store integrity is trusted there until then.
    ///
    /// Memory residual: the Git process owner buffers the whole blob in
    /// memory (at most 64 MiB plus slack) — a bounded transient allocation.
    ///
    /// On ANY error the sink may hold partial bytes; the caller discards it.
    pub async fn capture_head_blob(
        service: &GitService,
        repository: &GitRepository,
        entry: &SnapshotFileEntry,
        remaining_aggregate_bytes: u64,
        sink: &mut dyn Write,
        cancellation: &CancellationToken,
    ) -> Result<CapturedFile, SnapshotError> {
        let object = match entry.source() {
            SnapshotContentSource::GitBlob { object_id } => object_id,
            _ => return Err(SnapshotError::SourceNotGitBlob),
        };
        let declared = entry.declared_bytes();
        if declared > MAX_SNAPSHOT_FILE_BYTES || declared > remaining_aggregate_bytes {
            return Err(SnapshotError::TooLarge);
        }
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let deadline = Instant::now() + SNAPSHOT_BLOB_DEADLINE;
        let bytes = service
            .read_snapshot_blob(repository, object, cancellation, deadline)
            .await?;
        let received = bytes.len() as u64;
        if received != declared {
            return Err(SnapshotError::BlobSizeMismatch);
        }
        if repository.object_format() == GitObjectFormat::Sha256
            && !sha256_object_matches(object.as_str(), &bytes)
        {
            return Err(SnapshotError::BlobDigestMismatch);
        }
        let sha256 = digest(&bytes)?;
        for chunk in bytes.chunks(CHUNK_BYTES) {
            if cancellation.is_cancelled() {
                return Err(SnapshotError::Cancelled);
            }
            sink.write_all(chunk)
                .map_err(|error| capture_io("write the captured blob chunk", error))?;
        }
        Ok(CapturedFile::new(sha256, received))
    }

    /// Whether the blob bytes are the object a SHA-256 repository assigns
    /// `object_id_hex`: the lowercase hex SHA-256 of Git's `blob <len>\0`
    /// object framing plus the blob bytes must equal the id. Pure, so tests
    /// can exercise the comparison without a Git process.
    fn sha256_object_matches(object_id_hex: &str, bytes: &[u8]) -> bool {
        let mut hasher = Sha256::new();
        hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
        hasher.update(bytes);
        let hex: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        hex == object_id_hex
    }

    #[cfg(test)]
    mod tests {
        use super::sha256_object_matches;

        // sha256("blob 5\0hello"), from `printf 'blob 5\0hello' | shasum -a 256`.
        const HELLO_BLOB_ID: &str =
            "8aec4e4876f854f688d0ebfc8f37598f38e5fd6903cccc850ca36591175aeb60";
        // sha256("blob 0\0"), from `printf 'blob 0\0' | shasum -a 256`.
        const EMPTY_BLOB_ID: &str =
            "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813";
        // sha256("hello") without the object framing, from
        // `printf 'hello' | shasum -a 256`.
        const RAW_HELLO_ID: &str =
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

        #[test]
        fn framed_bytes_match_the_expected_object_id() {
            assert!(sha256_object_matches(HELLO_BLOB_ID, b"hello"));
        }

        #[test]
        fn one_bit_flip_mismatches() {
            // 'e' (0x65) -> 'd' (0x64) flips exactly one bit.
            assert!(!sha256_object_matches(HELLO_BLOB_ID, b"hdllo"));
        }

        #[test]
        fn same_length_different_content_mismatches() {
            assert!(!sha256_object_matches(HELLO_BLOB_ID, b"jello"));
        }

        #[test]
        fn missing_length_framing_mismatches() {
            assert!(!sha256_object_matches(RAW_HELLO_ID, b"hello"));
        }

        #[test]
        fn empty_blob_matches_its_known_object_id() {
            assert!(sha256_object_matches(EMPTY_BLOB_ID, b""));
        }
    }
}

#[cfg(unix)]
pub use unix_blob::capture_head_blob;
