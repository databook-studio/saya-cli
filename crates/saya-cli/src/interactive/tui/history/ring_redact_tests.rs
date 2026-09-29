//! `History::push` is the persisted-copy boundary (D2): an investigation
//! line's `--param` values never enter the ring or the `input_history` file.

use super::*;
use std::path::PathBuf;

fn tmp_path(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("saya_ring_redact_{tag}_{n}.txt"))
}

#[test]
fn push_redacts_investigation_param_values_in_ring_and_file() {
    let p = tmp_path("param");
    let mut h = History::with_path(p.clone());
    h.push("/investigation run inv-1 --param label=synthetic-confidential-customer-922");
    assert_eq!(
        h.previous(),
        Some("/investigation run inv-1 --param label=…"),
        "the ring keeps the redacted entry"
    );
    let file = std::fs::read_to_string(&p).unwrap();
    assert!(
        !file.contains("synthetic-confidential-customer-922"),
        "the value must not persist in the history file: {file:?}"
    );
    assert!(
        file.contains("--param label=…"),
        "the file keeps the redacted form: {file:?}"
    );
    let _ = std::fs::remove_file(p);
}

#[test]
fn push_keeps_non_investigation_lines_unchanged() {
    let p = tmp_path("plain");
    let mut h = History::with_path(p.clone());
    h.push("/sql SELECT * FROM orders");
    assert_eq!(h.previous(), Some("/sql SELECT * FROM orders"));
    let _ = std::fs::remove_file(p);
}
