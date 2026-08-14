//! Tests for `.saya/contracts/*.toml` discovery and parsing (slice 6a).
//!
//! These exercise the `pub(crate)` discovery surface in-crate because it is
//! `pub(crate)`. They build a synthetic `.saya/contracts` tree under a
//! process-unique temp dir and assert on the parsed report. Paths in
//! assertions are relative to the project root we hand to `discover_contracts`,
//! never absolute.
//!
//! The file format is TOML (serde via the workspace-pinned `toml` crate), chosen
//! over YAML so an attacker-influenceable shared-repo file is never fed through
//! an unsafe-derived parser; the shape — `{ version, object, [[claims]] }`
//! with each claim `{ kind, value, column? }` — is unchanged from the original
//! YAML form, and `deny_unknown_fields` still makes a typo an error.

use super::{
    DiscoveryReport, MAX_BYTES_PER_FILE, MAX_CLAIMS_PER_FILE, MAX_FILES, TruncationBound,
    discover_contracts,
};
use saya_types::ClaimPayload;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const VALID: &str = "\
version = 1
object = \"analytics.public.orders\"
[[claims]]
kind = \"description\"
value = \"one row per shipped order\"
[[claims]]
kind = \"time-column\"
value = \"created_at\"
[[claims]]
kind = \"column-role\"
column = \"customer_id\"
value = \"identifier\"
";

fn temp_root(label: &str) -> PathBuf {
    // Process-unique, deterministically named, cleaned up at the end.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-discover-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn contracts_dir(project: &Path) -> PathBuf {
    project.join(".saya").join("contracts")
}

fn write(project: &Path, name: &str, body: &str) {
    let dir = contracts_dir(project);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(name), body).unwrap();
}

fn assert_no_absolute_path(report: &DiscoveryReport, root: &Path) {
    for c in &report.contracts {
        assert!(
            !c.source.is_absolute(),
            "absolute source leaked: {:?}",
            c.source
        );
    }
    for (p, reason) in &report.rejected {
        assert!(!p.is_absolute(), "absolute rejected path: {:?}", p);
        let reason_root = root.to_string_lossy().to_string();
        assert!(
            !reason.contains(&reason_root),
            "absolute root leaked into reason: {reason}"
        );
    }
}

#[test]
fn valid_file_parses_to_claims() {
    let root = temp_root("valid");
    write(&root, "orders.toml", VALID);
    let report = discover_contracts(&root).unwrap();
    assert_eq!(report.contracts.len(), 1);
    let c = &report.contracts[0];
    assert_eq!(c.source, Path::new(".saya/contracts/orders.toml"));
    assert_eq!(c.object.catalog, "analytics");
    assert_eq!(c.object.schema, "public");
    assert_eq!(c.object.object, "orders");
    assert_eq!(c.claims.len(), 3);
    assert!(matches!(c.claims[0], ClaimPayload::TableDescription { .. }));
    assert!(matches!(
        c.claims[1],
        ClaimPayload::DefaultTimeColumn { .. }
    ));
    assert!(matches!(c.claims[2], ClaimPayload::ColumnRole { .. }));
    assert!(report.rejected.is_empty());
    assert!(report.truncated_by.is_none());
    assert_no_absolute_path(&report, &root);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unknown_field_is_rejected_and_others_still_parse() {
    let root = temp_root("unknown");
    write(&root, "good.toml", VALID);
    write(
        &root,
        "bad.toml",
        "\
version = 1
object = \"a.b.c\"
[[claims]]
kind = \"description\"
value = \"ok\"
bogus = \"field\"
",
    );
    let report = discover_contracts(&root).unwrap();
    let names: Vec<String> = report
        .contracts
        .iter()
        .map(|c| c.source.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.contains(&"good.toml".into()),
        "good file dropped: {names:?}"
    );
    let rejected_names: Vec<String> = report
        .rejected
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        rejected_names.contains(&"bad.toml".into()),
        "bad file not rejected: {rejected_names:?}"
    );
    // The reason names the offending field — a silently ignored field is a
    // contract a human believes is in force and is not. toml's
    // `deny_unknown_fields` error is `unknown field `bogus`, expected ...`,
    // which names the field on a single line via `Error::message()`.
    let bad_reason = report
        .rejected
        .iter()
        .find(|(p, _)| p.file_name().unwrap() == "bad.toml")
        .map(|(_, r)| r.as_str())
        .unwrap();
    assert!(
        bad_reason.contains("bogus"),
        "reason must name the field: {bad_reason}"
    );
    assert_no_absolute_path(&report, &root);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unknown_version_names_supported_version() {
    let root = temp_root("version");
    write(
        &root,
        "v2.toml",
        "version = 2\nobject = \"a.b.c\"\nclaims = []\n",
    );
    let report = discover_contracts(&root).unwrap();
    assert_eq!(report.contracts.len(), 0);
    assert_eq!(report.rejected.len(), 1);
    let reason = &report.rejected[0].1;
    assert!(
        reason.contains('1'),
        "reason must name supported version 1: {reason}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn malformed_qualified_name_is_rejected_no_completion() {
    let root = temp_root("malformed-q");
    write(
        &root,
        "partial.toml",
        "version = 1\nobject = \"orders\"\nclaims = []\n",
    );
    let report = discover_contracts(&root).unwrap();
    assert_eq!(report.contracts.len(), 0);
    assert_eq!(report.rejected.len(), 1);
    // A partial name must NOT be completed into a guessed object.
    assert!(report.contracts.is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn symlink_escaping_root_is_rejected() {
    // A symlink inside .saya/contracts pointing OUTSIDE the root must be
    // rejected, not followed. The canonical target is outside the canonical
    // root, so the candidate fails the inside-root check and is named.
    let root = temp_root("symlink-out");
    let outside = std::env::temp_dir().join(format!(
        "saya-discover-outside-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("evil.toml"), VALID).unwrap();
    let dir = contracts_dir(&root);
    fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(outside.join("evil.toml"), dir.join("link.toml")).unwrap();
    }
    let report = discover_contracts(&root).unwrap();
    let escaped = !cfg!(unix)
        || report
            .rejected
            .iter()
            .any(|(p, _)| p.file_name().map(|f| f == "link.toml").unwrap_or(false));
    assert!(escaped, "escaping symlink was followed, not rejected");
    // And the contract content must not have been imported.
    assert!(
        report.contracts.iter().all(|c| c.object.object != "orders") || !cfg!(unix),
        "escaping symlink content leaked into contracts"
    );
    assert_no_absolute_path(&report, &root);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn symlink_inside_root_is_followed() {
    // "Within the root" means within `.saya/contracts` (spec §4: the root IS
    // `.saya/contracts`). A symlink whose target is inside the contracts dir
    // is followed normally; one whose target is merely inside the *project*
    // (but outside `.saya/contracts`) escapes the contracts root and is
    // rejected — that is the previous test.
    let root = temp_root("symlink-in");
    let dir = contracts_dir(&root);
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("sub/real.toml"), VALID).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(dir.join("sub/real.toml"), dir.join("link.toml")).unwrap();
    }
    let report = discover_contracts(&root).unwrap();
    if cfg!(unix) {
        assert!(
            report.contracts.iter().any(|c| c.object.object == "orders"),
            "in-root symlink was not followed: {:?}",
            report.rejected
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn file_over_64kib_is_rejected_and_total_over_1mib_truncates() {
    let root = temp_root("oversize");
    // One file just over the per-file cap -> rejected, not parsed.
    let big = format!(
        "version = 1\nobject = \"a.b.c\"\n[[claims]]\nkind = \"description\"\nvalue = \"{}\"\n",
        "x".repeat(MAX_BYTES_PER_FILE)
    );
    write(&root, "big.toml", &big);
    let report = discover_contracts(&root).unwrap();
    assert!(
        report
            .rejected
            .iter()
            .any(|(p, _)| p.file_name().unwrap() == "big.toml"),
        "oversize file not rejected"
    );
    assert_eq!(report.contracts.len(), 0);

    // Now several files under the per-file cap whose total exceeds 1 MiB stops
    // the pass with truncated_by set. 20 files of ~56 KiB each = ~1.1 MiB,
    // under the 64 KiB per-file cap and under the 32-file cap, so the *total*
    // bound is what trips.
    let root = temp_root("oversize-total");
    let body = format!(
        "version = 1\nobject = \"a.b.c\"\n[[claims]]\nkind = \"description\"\nvalue = \"{}\"\n",
        "x".repeat(56 * 1024)
    );
    for i in 0..20 {
        write(&root, &format!("f{i:02}.toml"), &body);
    }
    let report = discover_contracts(&root).unwrap();
    assert!(
        report.truncated_by.is_some(),
        "total over 1 MiB did not truncate"
    );
    assert_eq!(
        report.truncated_by,
        Some(TruncationBound::TotalBytes.as_str())
    );
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn more_than_32_files_truncates() {
    let root = temp_root("many-files");
    for i in 0..(MAX_FILES + 5) {
        write(&root, &format!("f{i:02}.toml"), VALID);
    }
    let report = discover_contracts(&root).unwrap();
    assert!(
        report.truncated_by.is_some(),
        "over 32 files did not truncate"
    );
    assert_eq!(report.truncated_by, Some(TruncationBound::Files.as_str()));
    // No more than MAX_FILES contracts survived.
    assert!(report.contracts.len() <= MAX_FILES);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn missing_contracts_dir_is_empty_report() {
    let root = temp_root("missing");
    // No .saya at all.
    let report = discover_contracts(&root).unwrap();
    assert!(report.contracts.is_empty());
    assert!(report.rejected.is_empty());
    assert!(report.truncated_by.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn directory_named_toml_is_skipped() {
    let root = temp_root("dir-as-toml");
    let dir = contracts_dir(&root);
    fs::create_dir_all(&dir).unwrap();
    fs::create_dir_all(dir.join("x.toml")).unwrap();
    fs::write(dir.join("real.toml"), VALID).unwrap();
    let report = discover_contracts(&root).unwrap();
    assert!(
        report.contracts.iter().any(|c| c.object.object == "orders"),
        "real file beside a directory named x.toml was dropped"
    );
    // The directory is not a rejection — it is simply skipped.
    assert!(
        !report
            .rejected
            .iter()
            .any(|(p, _)| p.file_name().unwrap() == "x.toml"),
        "a directory was reported as a rejected file"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn control_character_value_is_rejected_by_constructor() {
    // The TOML parses fine (a quoted string may hold an escaped control char);
    // the claim value then goes through build_payload's constructor, which
    // rejects control characters. This is the test that proves we reuse the
    // constructors rather than deserializing ClaimPayload directly (a direct
    // deserialize would admit the control char).
    let root = temp_root("ctrl");
    let body = "version = 1\nobject = \"a.b.c\"\n[[claims]]\nkind = \"description\"\nvalue = \"hello\\nworld\"\n";
    write(&root, "ctrl.toml", body);
    let report = discover_contracts(&root).unwrap();
    assert_eq!(report.contracts.len(), 0);
    assert_eq!(report.rejected.len(), 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn more_than_128_claims_rejects_the_file() {
    // Claims-per-file is a per-file bound, like bytes-per-file: the offending
    // file is rejected and named, and the pass continues. Only the pass-wide
    // bounds (files, total-bytes) stop discovery. A file with one too many
    // claims is rejected and a sibling still parses.
    let root = temp_root("many-claims");
    let mut body = String::from("version = 1\nobject = \"a.b.c\"\n");
    for i in 0..(MAX_CLAIMS_PER_FILE + 5) {
        body.push_str(&format!(
            "[[claims]]\nkind = \"description\"\nvalue = \"claim-{i}\"\n"
        ));
    }
    write(&root, "many.toml", &body);
    write(&root, "ok.toml", VALID);
    let report = discover_contracts(&root).unwrap();
    assert_eq!(
        report.contracts.len(),
        1,
        "sibling was dropped: {:?}",
        report.rejected
    );
    assert!(
        report
            .rejected
            .iter()
            .any(|(p, _)| p.file_name().unwrap() == "many.toml"),
        "over-128-claims file was not rejected: {:?}",
        report.rejected
    );
    assert!(
        report.truncated_by.is_none(),
        "per-file claims bound must not stop the pass"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn non_toml_file_is_ignored() {
    let root = temp_root("non-toml");
    let dir = contracts_dir(&root);
    fs::create_dir_all(&dir).unwrap();
    // A contract body with a YAML/other extension must be ignored entirely —
    // the extension filter rejects it before parsing, so even valid content
    // under a `.yaml` name is not discovered.
    fs::write(dir.join("orders.yaml"), VALID).unwrap();
    fs::write(dir.join("README.md"), VALID).unwrap();
    let report = discover_contracts(&root).unwrap();
    assert!(report.contracts.is_empty());
    assert!(report.rejected.is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn fifo_named_toml_is_skipped_not_blocked() {
    // A FIFO opened for reading with no writer blocks forever — a DoS that
    // looks like a hang (spec §4). A FIFO named `x.toml` must be skipped
    // without ever being opened, so this test returning at all is the proof.
    // A blocking implementation would hang here and time out.
    let root = temp_root("fifo");
    let dir = contracts_dir(&root);
    fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        // Create a named FIFO at .saya/contracts/plug.toml.
        let fifo = dir.join("plug.toml");
        let rc = unsafe {
            libc::mkfifo(
                std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())
                    .unwrap()
                    .as_ptr(),
                0o600,
            )
        };
        assert_eq!(rc, 0, "mkfifo failed");
        // Sanity: it really is a FIFO, so the test is meaningful.
        assert!(fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo());
        // A real file beside it, to prove the pass still continues.
        fs::write(dir.join("real.toml"), VALID).unwrap();
        let report = discover_contracts(&root).unwrap();
        assert!(
            report.contracts.iter().any(|c| c.object.object == "orders"),
            "pass did not survive a FIFO sibling: {:?}",
            report.rejected
        );
        assert!(
            !report
                .rejected
                .iter()
                .any(|(p, _)| p.file_name().unwrap() == "plug.toml"),
            "a FIFO was reported as a rejected file rather than skipped"
        );
    }
    let _ = fs::remove_dir_all(&root);
}
