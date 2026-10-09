//! `redact_counted`: the counting variant.

use crate::redaction::{redact, redact_counted};

/// `redact_counted`'s text half is byte-identical to `redact`'s output for
/// every rule family — the counting variant must never change what gets
/// redacted, only report how many replacements happened.
#[test]
fn redact_counted_text_matches_redact_for_every_rule_family() {
    let samples = [
        "token=abc123",
        "password=hunter2 api_key=xyz",
        "Authorization: Bearer sk-live-abc123",
        "https://user:pass@host.example/path",
        "SAYA_RUN_EP_ORCHESTRATOR=sk-live-orchestrator",
        "no secrets here at all",
        "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----",
    ];
    for sample in samples {
        let (counted_text, _) = redact_counted(sample);
        assert_eq!(
            counted_text,
            redact(sample),
            "redact_counted's text must equal redact's output for {sample:?}"
        );
    }
}

/// The `key=value` marker family: one marker is one count.
#[test]
fn redact_counted_counts_key_value_markers() {
    assert_eq!(
        redact_counted("token=abc123"),
        ("token=[redacted]".to_string(), 1)
    );
    assert_eq!(
        redact_counted("password=a token=b"),
        ("password=[redacted] token=[redacted]".to_string(), 2)
    );
}

/// The credential-header family: one header line is one count, independent
/// of the marker family.
#[test]
fn redact_counted_counts_credential_headers() {
    assert_eq!(
        redact_counted("Authorization: Bearer sk-live-abc123"),
        ("Authorization: [redacted]".to_string(), 1)
    );
}

/// The URL user-info family: one `user:pass@` is one count.
#[test]
fn redact_counted_counts_url_userinfo() {
    let (text, count) = redact_counted("https://user:pass@host.example/path");
    assert_eq!(count, 1);
    assert_eq!(text, "https://[redacted]@host.example/path");
}

/// The env-assignment family (the harness's own credential shape) counts
/// too, and rule families sum across a mixed input.
#[test]
fn redact_counted_sums_across_rule_families() {
    let mixed = "SAYA_RUN_EP_REVIEWER=sk-1 token=abc Authorization: Bearer x";
    let (_, count) = redact_counted(mixed);
    assert_eq!(count, 3, "one hit per family must sum to 3");
}

/// Nothing to redact: zero count, and the text is unchanged — the pin the
/// model-context caller relies on to decide whether to show a note at all.
#[test]
fn redact_counted_is_zero_when_nothing_is_redacted() {
    let clean = "SELECT 1 WHERE status='done'";
    assert_eq!(redact_counted(clean), (clean.to_string(), 0));
}
