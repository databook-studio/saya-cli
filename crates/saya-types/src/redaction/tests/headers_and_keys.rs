//! The wholesale passes: credential headers redacted to end of line and PEM
//! private-key blocks removed wholesale (failed closed when truncated).

use crate::redaction::redact;

#[test]
fn credential_headers_redact_their_values_to_eol() {
    assert_eq!(
        redact("Authorization: Bearer sk-live-abc123"),
        "Authorization: [redacted]"
    );
    assert_eq!(redact("X-API-Key: hunter2 extra"), "X-API-Key: [redacted]");
    assert_eq!(redact("Cookie: session=xyz; path=/"), "Cookie: [redacted]");
    // Ordinary lines pass through untouched.
    assert_eq!(
        redact("SELECT 1 -- Authorization"),
        "SELECT 1 -- Authorization"
    );
}

#[test]
fn credential_header_inside_a_quoted_shell_argument_is_redacted() {
    // Row 1 of the spec table: a pasted `curl -H '...'` carries the header
    // mid-line, not at the start.
    let out = redact("curl -H 'Authorization: Bearer sk-live-SECRET' https://x");
    assert!(
        !out.contains("sk-live-SECRET"),
        "live token survived redaction: {out:?}"
    );
    assert!(
        out.contains("[redacted]"),
        "header value was not redacted: {out:?}"
    );
}

#[test]
fn truncated_private_key_block_is_redacted_to_end_of_buffer() {
    // Row 2 of the spec table: a BEGIN with no matching END must be redacted
    // from the BEGIN marker to the end of the buffer, not emitted verbatim.
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowSECRET\nmore";
    let out = redact(pem);
    assert!(
        !out.contains("MIIEowSECRET"),
        "truncated key body survived: {out:?}"
    );
    assert!(
        !out.contains("more"),
        "truncated key tail survived: {out:?}"
    );
    assert!(
        out.contains("[redacted private key]"),
        "no redaction marker emitted: {out:?}"
    );
}

#[test]
fn redacted_header_keeps_its_closing_bracket_across_newline() {
    // Row 3 of the spec table: the `[redacted]` must stay well-formed when
    // the header line is followed by more input.
    let out = redact("Authorization: Bearer x\nnext line");
    assert_eq!(out, "Authorization: [redacted]\nnext line");
}

#[test]
fn truncated_private_key_emits_nothing_after_begin_marker() {
    // Specifically: a PEM block with a PRIVATE KEY BEGIN and no
    // closing marker leaks nothing after the BEGIN marker.
    let pem = "before\n-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIEowSECRET\ntail-without-end";
    let out = redact(pem);
    assert!(out.contains("before"), "non-secret prefix lost: {out:?}");
    assert!(
        !out.contains("MIIEowSECRET"),
        "key body leaked after BEGIN marker: {out:?}"
    );
    assert!(
        !out.contains("tail-without-end"),
        "key tail leaked after BEGIN marker: {out:?}"
    );
    assert!(out.contains("[redacted private key]"));
}

#[test]
fn certificate_blocks_and_authorization_prose_stay_intact() {
    // Non-secret content is never destroyed.
    let cert = "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----";
    assert_eq!(redact(cert), cert);
    assert_eq!(
        redact("SELECT 1 -- Authorization"),
        "SELECT 1 -- Authorization"
    );
}

#[test]
fn pem_private_key_blocks_are_removed_wholesale() {
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAK\nabcdef==\n-----END RSA PRIVATE KEY-----\nafter";
    let out = redact(pem);
    assert!(!out.contains("MIIEow"));
    assert!(out.contains("[redacted private key]"));
    assert!(out.contains("after"));
    // Public certs are left alone (not secret material).
    let cert = "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----";
    assert_eq!(redact(cert), cert);
}
