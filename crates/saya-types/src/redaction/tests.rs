use super::{CREDENTIAL_ENV_PREFIX, redact};

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

// -- the env-assignment pass: the credential shapes this product creates ----

#[test]
fn the_generated_endpoint_env_assignment_is_redacted() {
    // The exact variable name the run-config generator binds
    // (`api_key = { env = "SAYA_RUN_EP_<ROLE>" }`, asserted in
    // saya-harness's run_configs battery) — none of the `key=value` markers
    // match it, which is the hole this pass closes.
    assert_eq!(CREDENTIAL_ENV_PREFIX, "SAYA_RUN_EP_");
    assert_eq!(
        redact("SAYA_RUN_EP_ORCHESTRATOR=sk-live-orchestrator"),
        "SAYA_RUN_EP_ORCHESTRATOR=[redacted]"
    );
}

#[test]
fn an_env_dump_line_is_redacted_per_variable() {
    // The shape a child echoing its environment produces: one `NAME=value`
    // per line, neighbours untouched.
    let dump = "PATH=/usr/bin:/bin\nSAYA_RUN_EP_REVIEW_1=sk-live-123\nHOME=/Users/x";
    let out = redact(dump);
    assert!(out.contains("PATH=/usr/bin:/bin"), "{out:?}");
    assert!(out.contains("SAYA_RUN_EP_REVIEW_1=[redacted]"), "{out:?}");
    assert!(out.contains("HOME=/Users/x"), "{out:?}");
    assert!(!out.contains("sk-live-123"), "{out:?}");
}

#[test]
fn credential_named_assignments_are_redacted_inline() {
    assert_eq!(
        redact("export DB_PASSWORD=hunter2; echo ok"),
        "export DB_PASSWORD=[redacted]; echo ok"
    );
    assert_eq!(redact("AUTH_TOKEN=abc def"), "AUTH_TOKEN=[redacted] def");
    assert_eq!(redact("MY_SECRETS=x"), "MY_SECRETS=[redacted]");
    assert_eq!(redact("APIKEY=z"), "APIKEY=[redacted]");
    assert_eq!(redact("PG_PASSWD=x y"), "PG_PASSWD=[redacted] y");
    assert_eq!(
        redact("SERVICE_CREDENTIALS=q r"),
        "SERVICE_CREDENTIALS=[redacted] r"
    );
    // Quotes are part of the redacted value, not a gap after it.
    assert_eq!(redact("KEY=\"sk-a\" tail"), "KEY=[redacted] tail");
    // An inline error message quoting the assignment:
    assert_eq!(
        redact("error: SAYA_RUN_EP_REVIEWER=sk-1 refused"),
        "error: SAYA_RUN_EP_REVIEWER=[redacted] refused"
    );
}

#[test]
fn non_credential_names_and_ordinary_comparisons_pass_through() {
    let sql = "SELECT 1 WHERE status='done' AND count=42";
    assert_eq!(redact(sql), sql);
    assert_eq!(redact("PATH=/usr/bin:/bin"), "PATH=/usr/bin:/bin");
    // Words that merely contain a credential suffix never match: the word
    // must be the whole name or follow a `_`/`-` boundary.
    assert_eq!(redact("monkey=banana"), "monkey=banana");
    assert_eq!(redact("oauth=https://x.example"), "oauth=https://x.example");
}

#[test]
fn the_prefix_rule_is_case_insensitive_and_shared_with_the_generator() {
    assert_eq!(
        redact("saya_run_ep_orchestrator=sk-1"),
        "saya_run_ep_orchestrator=[redacted]"
    );
    // The harness's endpoint_env_var must build names under the same
    // constant; the escape battery's credential test runs the real
    // generated name through the capture path to prove they cannot drift.
    let composed = format!("{CREDENTIAL_ENV_PREFIX}REVIEWER=sk-2");
    assert_eq!(redact(&composed), "SAYA_RUN_EP_REVIEWER=[redacted]");
}

#[test]
fn the_original_key_value_markers_are_unchanged() {
    assert_eq!(redact("password=hunter2"), "password=[redacted]");
    assert_eq!(redact("api_key=abc&next=1"), "api_key=[redacted]&next=1");
}
