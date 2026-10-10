//! Cursor reads over the private output store: a [`CommandOutputCursor`]
//! names a stream and a byte offset, and [`CommandOutputStore::read`]
//! serves at most [`MAX_OUTPUT_READ_BYTES`] stored bytes from there as one
//! [`CommandOutputRead`]. The text is the lossy-UTF-8 view of the stored
//! window (`String::from_utf8_lossy`): a window may cut a multi-byte
//! character, and the cursor advances by bytes read, so resuming never
//! skips or re-reads a byte.
//!
//! `end_of_stream` is reported only on a finished stream read at the
//! stored length: before finish the writer may still append. While the
//! stream runs the accounting is atomic (raw totals, channel-rejected
//! drops, `truncated: false`); after finish it is the exact finish result.
//! Reads open no-follow through the Unix seam: a store removed by
//! retention is `Closed`; other failures are typed `Store` errors — never
//! message text, never paths.
//!
//! A writer detached after a bounded-finish timeout (see
//! [`super::output_writer`]) can append after the stream reports finished,
//! so `end_of_stream` can be early for such a stream; counters for it are
//! frozen at zero.

use std::fmt;

#[cfg(unix)]
use std::io::{self, Read, Seek, SeekFrom};

use super::output::{CommandOutputError, CommandOutputStore, MAX_OUTPUT_READ_BYTES, OutputStream};

#[cfg(unix)]
use super::output_store_unix::{open_read_file, store_error};

/// A position in one stored stream: the stream and a byte offset. `Debug`
/// shows only the stream and offset, never a path.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CommandOutputCursor {
    stream: OutputStream,
    offset: u64,
}

impl fmt::Debug for CommandOutputCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandOutputCursor")
            .field("stream", &self.stream)
            .field("offset", &self.offset)
            .finish()
    }
}

impl CommandOutputCursor {
    /// The cursor at the start of `stream`'s stored file.
    pub fn start(stream: OutputStream) -> Self {
        Self { stream, offset: 0 }
    }

    /// The stream this cursor reads.
    pub fn stream(self) -> OutputStream {
        self.stream
    }

    /// The byte offset this cursor reads from.
    pub fn offset(self) -> u64 {
        self.offset
    }
}

/// One bounded read: lossy-UTF-8 `text` of the stored window, where to
/// resume, and the stream's accounting. `text` is redacted store content:
/// stored output is private even after redaction, so `Debug` reports only
/// its byte length, never the text.
#[derive(Clone, PartialEq, Eq)]
pub struct CommandOutputRead {
    next_cursor: CommandOutputCursor,
    text: String,
    truncated: bool,
    dropped_bytes: u64,
    raw_total_bytes: u64,
    end_of_stream: bool,
}

impl fmt::Debug for CommandOutputRead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandOutputRead")
            .field("next_cursor", &self.next_cursor)
            .field("text_bytes", &self.text.len())
            .field("truncated", &self.truncated)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("raw_total_bytes", &self.raw_total_bytes)
            .field("end_of_stream", &self.end_of_stream)
            .finish()
    }
}

impl CommandOutputRead {
    /// Where the next read continues: requested offset plus bytes read.
    pub fn next_cursor(&self) -> CommandOutputCursor {
        self.next_cursor
    }

    /// The lossy-UTF-8 view of the stored window this read covered.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the stream hit its cap; exact after finish, `false` before.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Raw bytes counted dropped; while running this undercounts drops the
    /// writer adds only at finish.
    pub fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes
    }

    /// Every raw byte the child produced on this stream, stored or dropped.
    pub fn raw_total_bytes(&self) -> u64 {
        self.raw_total_bytes
    }

    /// True only on a finished stream read at the stored length.
    pub fn end_of_stream(&self) -> bool {
        self.end_of_stream
    }
}

impl CommandOutputStore {
    /// Serves at most `max_bytes` stored bytes from `cursor`. Zero is an
    /// empty read at the cursor; past [`MAX_OUTPUT_READ_BYTES`] is
    /// [`CommandOutputError::ReadTooLarge`].
    pub(super) fn read(
        &self,
        cursor: CommandOutputCursor,
        max_bytes: usize,
    ) -> Result<CommandOutputRead, CommandOutputError> {
        if max_bytes > MAX_OUTPUT_READ_BYTES {
            return Err(CommandOutputError::ReadTooLarge { max_bytes });
        }
        self.read_bounded(cursor, max_bytes)
    }

    /// Unix read path: open no-follow, verify a regular file on the opened
    /// descriptor, bound the offset against the stored length, then one
    /// take-bounded read.
    #[cfg(unix)]
    fn read_bounded(
        &self,
        cursor: CommandOutputCursor,
        max_bytes: usize,
    ) -> Result<CommandOutputRead, CommandOutputError> {
        let file = open_read_file(self.dir(), cursor.stream()).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                CommandOutputError::Closed
            } else {
                store_error(error)
            }
        })?;
        let sink = self.sink(cursor.stream());
        // Queried before the length: a finished writer appends no more, so
        // the length below is final; a finish racing in later only delays
        // `end_of_stream`, it never fakes one.
        let finished = sink.result();
        let metadata = file.metadata().map_err(store_error)?;
        if !metadata.is_file() {
            return Err(CommandOutputError::Store {
                kind: io::ErrorKind::InvalidInput,
                os_code: None,
            });
        }
        let stored_len = metadata.len();
        if cursor.offset() > stored_len {
            return Err(CommandOutputError::CursorInvalid {
                offset: cursor.offset(),
            });
        }
        let mut file = file;
        file.seek(SeekFrom::Start(cursor.offset()))
            .map_err(store_error)?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(max_bytes as u64)
            .read_to_end(&mut bytes)
            .map_err(store_error)?;
        let (raw_total, dropped, truncated) = match &finished {
            Some(result) => (result.raw_total_bytes, result.dropped_bytes, result.capped),
            None => (
                sink.running_raw_total_bytes(),
                sink.running_dropped_bytes(),
                false,
            ),
        };
        let next_cursor = CommandOutputCursor {
            stream: cursor.stream(),
            offset: cursor.offset().saturating_add(bytes.len() as u64),
        };
        Ok(CommandOutputRead {
            next_cursor,
            text: String::from_utf8_lossy(&bytes).into_owned(),
            truncated,
            dropped_bytes: dropped,
            raw_total_bytes: raw_total,
            end_of_stream: finished.is_some() && cursor.offset() == stored_len,
        })
    }

    /// The store is Unix-only: without the `openat` seam a store is never
    /// prepared, so a read is typed `Unavailable`.
    #[cfg(not(unix))]
    fn read_bounded(
        &self,
        _cursor: CommandOutputCursor,
        _max_bytes: usize,
    ) -> Result<CommandOutputRead, CommandOutputError> {
        Err(CommandOutputError::Unavailable)
    }
}
