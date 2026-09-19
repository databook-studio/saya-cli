use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const MAX_ENTRY_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 1000;

/// Persistent, de-duplicated input history with Up/Down navigation.
#[allow(dead_code)]
pub(crate) struct History {
    entries: Vec<String>,
    cursor: Option<usize>,
    path: PathBuf,
    limit: usize,
    disabled: bool,
    omitted: usize,
}

fn is_disabled_env() -> bool {
    std::env::var("SAYA_HISTORY").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false"
        )
    })
}

#[allow(dead_code)]
impl History {
    /// Entries matching `needle` (case-insensitive substring), newest first.
    pub(crate) fn search(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|entry| entry.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }

    pub(crate) fn load() -> Self {
        let path = crate::interactive::session_paths::default_history_file();
        let disabled = is_disabled_env();
        Self::from_path(path, disabled)
    }

    pub(crate) fn with_path(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            path,
            limit: MAX_ENTRIES,
            disabled: false,
            omitted: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_path_disabled(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            path,
            limit: MAX_ENTRIES,
            disabled: true,
            omitted: 0,
        }
    }

    /// Number of entries omitted by the safety bounds since this history was loaded.
    pub(crate) fn omitted_count(&self) -> usize {
        self.omitted
    }

    pub(crate) fn push(&mut self, line: &str) -> bool {
        if self.disabled {
            return false;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || self.entries.last().map(String::as_str) == Some(trimmed) {
            return false;
        }
        if trimmed.len() > MAX_ENTRY_BYTES {
            self.omitted += 1;
            return false;
        }
        self.cursor = None;
        self.entries.push(trimmed.to_string());
        if self.entries.len() > self.limit {
            let removed = self.entries.len() - self.limit;
            self.entries.drain(..removed);
            self.omitted += removed;
        }
        while total_bytes(&self.entries) > MAX_TOTAL_BYTES {
            self.entries.remove(0);
            self.omitted += 1;
        }
        self.save();
        true
    }

    pub(crate) fn previous(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = self
            .cursor
            .map_or(self.entries.len() - 1, |i| i.saturating_sub(1));
        self.cursor = Some(idx);
        Some(&self.entries[idx])
    }

    pub(crate) fn next(&mut self) -> Option<&str> {
        let idx = self.cursor?;
        if idx >= self.entries.len().checked_sub(1)? {
            self.cursor = None;
            None
        } else {
            self.cursor = Some(idx + 1);
            Some(&self.entries[idx + 1])
        }
    }

    pub(crate) fn reset(&mut self) {
        self.cursor = None;
    }

    fn save(&self) {
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
    fn from_path(path: PathBuf, disabled: bool) -> Self {
        let (entries, omitted) = if disabled {
            (Vec::new(), 0)
        } else {
            load_entries(&path)
        };
        Self {
            entries,
            cursor: None,
            path,
            limit: MAX_ENTRIES,
            disabled,
            omitted,
        }
    }
}

fn total_bytes(entries: &[String]) -> usize {
    entries.iter().map(String::len).sum::<usize>() + entries.len().saturating_sub(1)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("saya_hist_{tag}_{n}.txt"))
    }

    #[test]
    fn test_navigation_clamping_and_dedup() {
        let p = tmp_path("nav");
        let mut h = History::with_path(p.clone());
        assert_eq!(h.previous(), None);
        assert_eq!(h.next(), None);
        h.push("first");
        h.push("second");
        h.push("second");
        h.push("third");
        assert_eq!(h.previous(), Some("third"));
        assert_eq!(h.previous(), Some("second"));
        assert_eq!(h.previous(), Some("first"));
        assert_eq!(h.previous(), Some("first"));
        assert_eq!(h.next(), Some("second"));
        assert_eq!(h.next(), Some("third"));
        assert_eq!(h.next(), None);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn test_redaction_and_persistence() {
        let p = tmp_path("redact");
        let raw = "connect password=hunter2 token=abc";
        let mut h = History::with_path(p.clone());
        h.push("one");
        h.push(raw);
        assert_eq!(h.previous(), Some(raw));
        h.reset();
        assert_eq!(h.cursor, None);
        let c = std::fs::read_to_string(&p).unwrap();
        assert!(
            c.contains("one\n")
                && c.contains("[redacted]")
                && !c.contains("hunter2")
                && !c.contains("abc")
        );
        let _ = std::fs::remove_file(p);
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp_path("perms");
        let mut h = History::with_path(p.clone());
        h.push("line");
        let meta = std::fs::metadata(&p).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn test_disabled_history() {
        let p = tmp_path("disabled");
        let mut h = History::with_path_disabled(p.clone());
        h.push("secret entry");
        assert!(!p.exists() && h.previous().is_none());
    }

    #[test]
    fn test_atomic_and_complete() {
        let p = tmp_path("atomic");
        let mut h = History::with_path(p.clone());
        h.push("line1 password=secret1");
        h.push("line2 token=secret2");
        h.push("line3");
        let c = std::fs::read_to_string(&p).unwrap();
        let exp = vec![
            "line1 password=[redacted]",
            "line2 token=[redacted]",
            "line3",
        ];
        assert_eq!(c.lines().collect::<Vec<_>>(), exp);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn oversized_entry_is_omitted_and_reported() {
        let p = tmp_path("entry_bound");
        let mut h = History::with_path(p.clone());
        assert!(!h.push(&"x".repeat(MAX_ENTRY_BYTES + 1)));
        assert_eq!(h.omitted_count(), 1);
        assert!(h.previous().is_none());
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn total_bound_keeps_the_newest_entries() {
        let p = tmp_path("total_bound");
        let mut h = History::with_path(p.clone());
        let entry = "x".repeat(MAX_ENTRY_BYTES - 16);
        for index in 0..20 {
            h.push(&format!("{index:02}-{entry}"));
        }

        assert!(h.omitted_count() >= 1);
        assert!(h.entries.len() < 20);
        assert!(h.entries.last().is_some_and(|line| line.starts_with("19-")));
        assert!(h.previous().is_some_and(|line| line.starts_with("19-")));
        assert!(h.previous().is_some_and(|line| line.starts_with("18-")));
        assert!(std::fs::metadata(&p).unwrap().len() as usize <= MAX_TOTAL_BYTES);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn load_skips_oversized_and_partial_utf8_entries_but_keeps_newest() {
        let p = tmp_path("load_bound");
        let contents = format!(
            "{}\nold 🦀\nnew 🦀",
            "x".repeat(MAX_TOTAL_BYTES + MAX_ENTRY_BYTES)
        );
        std::fs::write(&p, contents).unwrap();

        let h = History::from_path(p.clone(), false);
        assert!(h.omitted_count() >= 1);
        assert_eq!(h.entries, ["old 🦀", "new 🦀"]);
        assert!(
            h.entries
                .iter()
                .all(|entry| std::str::from_utf8(entry.as_bytes()).is_ok())
        );
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    /// Entries are set directly rather than pushed: `push` calls `save`, and
    /// these tests only exercise `search`. Driving them through `push` wrote a
    /// `saya-test-history` file into the crate directory on every `cargo test`.
    fn history() -> History {
        History {
            entries: vec![
                "SELECT * FROM orders".to_string(),
                "explain select 1".to_string(),
                "select count(*) from events".to_string(),
            ],
            cursor: None,
            path: std::path::PathBuf::new(),
            limit: MAX_ENTRIES,
            disabled: true,
            omitted: 0,
        }
    }

    #[test]
    fn search_is_case_insensitive_and_newest_first() {
        let matches = history().search("SELECT");
        assert_eq!(
            matches,
            vec![
                "select count(*) from events",
                "explain select 1",
                "SELECT * FROM orders"
            ]
        );
        assert!(history().search("zzz").is_empty());
    }
}
