//! History persistence: atomic save with redaction, bounded load from disk.

use super::{History, MAX_ENTRIES, MAX_ENTRY_BYTES, MAX_TOTAL_BYTES, total_bytes};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

pub(super) fn is_disabled_env() -> bool {
    std::env::var("SAYA_HISTORY").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false"
        )
    })
}

#[allow(dead_code)]
impl History {
    pub(super) fn save(&self) {
        if self.disabled {
            return;
        }
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let pid = std::process::id();
        let name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("history");
        let tmp = self.path.with_file_name(format!("{name}.{pid}.tmp"));
        let mut lines = Vec::new();
        let mut content_bytes = 0;
        for entry in self.entries.iter().rev() {
            let redacted = saya_store::redact(entry);
            let separator = usize::from(!lines.is_empty());
            if redacted.len() > MAX_ENTRY_BYTES
                || content_bytes + separator + redacted.len() > MAX_TOTAL_BYTES
            {
                continue;
            }
            content_bytes += separator + redacted.len();
            lines.push(redacted);
        }
        lines.reverse();
        let content = lines.join("\n");

        let write_tmp = || -> std::io::Result<()> {
            #[cfg(unix)]
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            #[cfg(unix)]
            let mut opts = std::fs::OpenOptions::new();
            #[cfg(unix)]
            opts.mode(0o600);
            #[cfg(not(unix))]
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            let mut f = opts.open(&tmp)?;
            f.write_all(content.as_bytes())?;
            f.flush()?;
            drop(f);
            #[cfg(unix)]
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            std::fs::rename(&tmp, &self.path)
        };
        if write_tmp().is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

impl History {
    pub(super) fn from_path(path: PathBuf, disabled: bool) -> Self {
        let (entries, omitted) = if disabled {
            (Vec::new(), 0)
        } else {
            load_entries(&path)
        };
        Self {
            entries,
            cursor: None,
            stash: None,
            path,
            limit: MAX_ENTRIES,
            disabled,
            omitted,
        }
    }
}

fn load_entries(path: &std::path::Path) -> (Vec<String>, usize) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return (Vec::new(), 0);
    };
    let Ok(length) = file.metadata().map(|meta| meta.len()) else {
        return (Vec::new(), 0);
    };
    let start = length.saturating_sub(MAX_TOTAL_BYTES as u64);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (Vec::new(), 0);
    }
    let mut bytes = Vec::new();
    if file
        .take(MAX_TOTAL_BYTES as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return (Vec::new(), 0);
    }

    let truncated_prefix = start > 0;
    let mut lines = bytes.rsplit(|byte| *byte == b'\n').collect::<Vec<_>>();
    if truncated_prefix {
        // The first chunk may begin halfway through an old line. Never turn
        // that fragment into a recalled command.
        lines.pop();
    }
    let mut omitted = usize::from(truncated_prefix);
    let mut newest = Vec::new();
    for line in lines {
        let Ok(line) = std::str::from_utf8(line) else {
            omitted += 1;
            continue;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_ENTRY_BYTES {
            omitted += 1;
            continue;
        }
        if newest.len() >= MAX_ENTRIES
            || total_bytes(&newest) + line.len() + usize::from(!newest.is_empty()) > MAX_TOTAL_BYTES
        {
            omitted += 1;
            break;
        }
        newest.push(line.to_string());
    }
    newest.reverse();
    (newest, omitted)
}
