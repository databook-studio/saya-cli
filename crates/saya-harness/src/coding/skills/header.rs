use std::io::Read;
use std::os::unix::fs::MetadataExt;

use crate::{HarnessError, workspace::Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderReadError {
    Missing,
    TooLarge,
    Symlink,
    NotRegular,
    Unterminated,
    Refused,
    Unreadable,
}

/// Reads only through the closing frontmatter line. A single-byte read avoids
/// buffering body bytes past that boundary.
#[cfg(unix)]
pub(super) fn read(
    workspace: &Workspace,
    relative_path: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, HeaderReadError> {
    let anchor = workspace.anchor(relative_path, false).map_err(map_error)?;
    let Some(stat) = anchor.stat() else {
        return Err(HeaderReadError::Missing);
    };
    if stat.is_symlink() {
        return Err(HeaderReadError::Symlink);
    }
    if !stat.is_file() {
        return Err(HeaderReadError::NotRegular);
    }
    // A file can change from regular to a FIFO after the stat. Nonblocking
    // open lets the post-open type and identity checks reject that swap
    // before any header bytes are read; the anchored open adds no-follow and
    // close-on-exec flags.
    let file = anchor
        .open_file(libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY, 0)
        .map_err(map_open_error)?;
    let opened = file.metadata().map_err(|_| HeaderReadError::Unreadable)?;
    if !opened.is_file() {
        return Err(HeaderReadError::NotRegular);
    }
    if (opened.dev(), opened.ino()) != stat.identity() {
        return Err(HeaderReadError::Refused);
    }

    let mut bytes = Vec::new();
    let mut line_start = 0;
    let mut line_number = 0;
    loop {
        if bytes.len() == max_bytes {
            return Err(HeaderReadError::TooLarge);
        }
        let mut byte = [0_u8; 1];
        let count = (&file)
            .read(&mut byte)
            .map_err(|_| HeaderReadError::Unreadable)?;
        if count == 0 {
            return Err(HeaderReadError::Unterminated);
        }
        bytes.push(byte[0]);
        if line_number == 0 {
            let first_line_prefix = b"---";
            let index = bytes.len() - 1;
            if index < first_line_prefix.len() && byte[0] != first_line_prefix[index] {
                return Err(HeaderReadError::Unterminated);
            }
            if index == first_line_prefix.len() && !matches!(byte[0], b'\n' | b'\r') {
                return Err(HeaderReadError::Unterminated);
            }
        }
        if byte[0] == b'\n' {
            let line = bytes[line_start..bytes.len() - 1]
                .strip_suffix(b"\r")
                .unwrap_or(&bytes[line_start..bytes.len() - 1]);
            if line_number == 0 && line != b"---" {
                return Err(HeaderReadError::Unterminated);
            }
            if line_number > 0 && line == b"---" {
                return Ok(bytes);
            }
            line_number += 1;
            line_start = bytes.len();
        }
    }
}

fn map_error(error: HarnessError) -> HeaderReadError {
    match error {
        HarnessError::NotFound { .. } => HeaderReadError::Missing,
        HarnessError::SymlinkRefused { .. } => HeaderReadError::Symlink,
        HarnessError::NotRegularFile { .. } => HeaderReadError::NotRegular,
        _ => HeaderReadError::Refused,
    }
}

fn map_open_error(error: std::io::Error) -> HeaderReadError {
    if error.raw_os_error() == Some(libc::ELOOP) {
        HeaderReadError::Symlink
    } else if error.kind() == std::io::ErrorKind::NotFound {
        HeaderReadError::Missing
    } else {
        HeaderReadError::Refused
    }
}
