//! Unit tests for [`redact_param_values`]: every persisted/echoed copy of a
//! submitted command line shows `--param name=…` (D2), while lines that are
//! not investigation commands pass through unchanged.

use super::redact_param_values as redact;

#[test]
fn a_run_binding_is_redacted() {
    assert_eq!(
        redact("/investigation run inv-1 --param label=secret-922"),
        "/investigation run inv-1 --param label=…"
    );
}

#[test]
fn repeated_bindings_all_redact() {
    assert_eq!(
        redact("/investigation run inv-1 --param a=1 --param b=2"),
        "/investigation run inv-1 --param a=… --param b=…"
    );
}

#[test]
fn the_attached_equals_spelling_redacts() {
    assert_eq!(
        redact("/investigation run inv-1 --param=label=secret-922"),
        "/investigation run inv-1 --param=label=…"
    );
}

#[test]
fn a_quoted_value_with_spaces_redacts_to_the_next_flag() {
    assert_eq!(
        redact("/investigation run inv-1 --param label=\"two words\" --connection demo"),
        "/investigation run inv-1 --param label=… --connection demo"
    );
}

#[test]
fn an_unquoted_value_with_spaces_redacts_wholly() {
    assert_eq!(
        redact("/investigation run inv-1 --param label=two words"),
        "/investigation run inv-1 --param label=…"
    );
}

#[test]
fn a_binding_without_equals_redacts_wholly() {
    assert_eq!(
        redact("/investigation run inv-1 --param label"),
        "/investigation run inv-1 --param …"
    );
}

#[test]
fn a_bare_param_at_the_end_of_the_line_redacts() {
    assert_eq!(
        redact("/investigation run inv-1 --param"),
        "/investigation run inv-1 --param …"
    );
}

#[test]
fn a_value_after_a_second_equals_stays_hidden() {
    assert_eq!(
        redact("/investigation run inv-1 --param a=b=c"),
        "/investigation run inv-1 --param a=…"
    );
}

#[test]
fn an_empty_attached_binding_redacts() {
    assert_eq!(
        redact("/investigation run inv-1 --param="),
        "/investigation run inv-1 --param=…"
    );
}

#[test]
fn non_investigation_lines_are_unchanged() {
    assert_eq!(
        redact("/sql SELECT * FROM orders"),
        "/sql SELECT * FROM orders"
    );
    assert_eq!(
        redact("/sql SELECT '--param a=b'"),
        "/sql SELECT '--param a=b'"
    );
    assert_eq!(redact("run inv-1 --param a=b"), "run inv-1 --param a=b");
    assert_eq!(redact("/sessions"), "/sessions");
}

#[test]
fn the_investigations_alias_family_redacts() {
    assert_eq!(
        redact("/investigations run x --param a=b"),
        "/investigations run x --param a=…"
    );
}

#[test]
fn the_param_spec_flag_is_never_touched() {
    assert_eq!(
        redact("/investigation save x --param-spec label:string:required"),
        "/investigation save x --param-spec label:string:required"
    );
}

#[test]
fn original_whitespace_is_preserved() {
    assert_eq!(
        redact("  /investigation run x --param   a=b"),
        "  /investigation run x --param   a=…"
    );
}

#[test]
fn redaction_is_idempotent() {
    let line = "/investigation run inv-1 --param label=secret-922 --connection demo";
    let once = redact(line);
    assert_eq!(redact(&once), once);
}

#[test]
fn lines_without_param_are_unchanged() {
    assert_eq!(
        redact("/investigation run inv-1 --connection demo"),
        "/investigation run inv-1 --connection demo"
    );
}
