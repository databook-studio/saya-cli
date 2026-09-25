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
