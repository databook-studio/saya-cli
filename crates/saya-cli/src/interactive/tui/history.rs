use std::io::Write;
use std::path::PathBuf;

/// Persistent, de-duplicated input history with Up/Down navigation.
#[allow(dead_code)]
pub(crate) struct History {
    entries: Vec<String>,
    cursor: Option<usize>,
    path: PathBuf,
    limit: usize,
    disabled: bool,
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
        let entries = if disabled {
            Vec::new()
        } else {
            std::fs::read_to_string(&path)
                .map(|c| {
                    let mut l: Vec<_> = c
                        .lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if l.len() > 1000 {
                        l.drain(..l.len() - 1000);
                    }
                    l
                })
                .unwrap_or_default()
        };
        Self {
            entries,
            cursor: None,
            path,
            limit: 1000,
            disabled,
        }
    }

    pub(crate) fn with_path(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            path,
            limit: 1000,
            disabled: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_path_disabled(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            path,
            limit: 1000,
            disabled: true,
        }
    }

    pub(crate) fn push(&mut self, line: &str) {
        if self.disabled {
            return;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || self.entries.last().map(String::as_str) == Some(trimmed) {
            return;
        }
        self.cursor = None;
        self.entries.push(trimmed.to_string());
        if self.entries.len() > self.limit {
            self.entries.drain(..self.entries.len() - self.limit);
        }
        self.save();
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
        let content = self
            .entries
            .iter()
            .map(|e| saya_store::redact(e))
            .collect::<Vec<_>>()
            .join("\n");

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
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn history() -> History {
        let mut history = History {
            entries: Vec::new(),
            cursor: None,
            path: std::path::PathBuf::from("saya-test-history"),
            limit: 1000,
            disabled: false,
        };
        history.push("SELECT * FROM orders");
        history.push("explain select 1");
        history.push("select count(*) from events");
        history
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
