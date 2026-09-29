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

// --- R061-1/R061-2 regressions (audit 0610aaf5, D8) -------------------------

/// Redaction of `line` must be idempotent: the redacted line redacts to
/// itself, whatever shape the line had.
fn assert_idempotent(line: &str) {
    let once = redact(line);
    assert_eq!(
        redact(&once),
        once,
        "redaction must be idempotent: {line:?} -> {once:?}"
    );
}

/// R061-1: an attached-looking token inside an exact `--param` value zone is
/// value text (the parser's rule), so the zone consumes it and the walk must
/// never revisit it — the old code sliced backwards here and panicked.
#[test]
fn an_attached_looking_token_inside_a_value_zone_stays_hidden() {
    let line = "/investigation run inv-1 --param label=hello --param=x=y";
    let redacted = redact(line);
    assert_eq!(redacted, "/investigation run inv-1 --param label=…");
    assert!(
        !redacted.contains("hello"),
        "the value stays hidden: {redacted:?}"
    );
    assert!(
        !redacted.contains("x=y"),
        "the nested token stays hidden: {redacted:?}"
    );
    assert_idempotent(line);
}

/// R061-2: the attached spelling is rejected by the parser but history is
/// written first, so the whole remainder through the next exact run flag is
/// redacted — multiword values included, not just the first token.
#[test]
fn an_attached_form_redacts_the_whole_remainder() {
    let line = "/investigation run inv-1 --param=label=two confidential words";
    let redacted = redact(line);
    assert_eq!(redacted, "/investigation run inv-1 --param=label=…");
    assert!(
        !redacted.contains("confidential words"),
        "the trailing value text stays hidden: {redacted:?}"
    );
    assert_idempotent(line);
}

/// The audit's second panic line: mixed exact and attached-looking tokens,
/// followed by a later `--param` and `--connection`.
#[test]
fn mixed_exact_and_attached_tokens_redact_through_the_next_exact_flag() {
    let line =
        "/investigation run inv-1 --param label=hello --param=x=y --param z=next --connection demo";
    let redacted = redact(line);
    assert_eq!(
        redacted,
        "/investigation run inv-1 --param label=… --param z=… --connection demo"
    );
    assert!(
        !redacted.contains("hello"),
        "the first value stays hidden: {redacted:?}"
    );
    assert!(
        !redacted.contains("x=y"),
        "the nested token stays hidden: {redacted:?}"
    );
    assert!(
        !redacted.contains("next"),
        "the later binding's value stays hidden: {redacted:?}"
    );
    assert_idempotent(line);
}

/// Non-ASCII values survive the walk at byte offsets: no panic, same shape.
#[test]
fn unicode_values_redact_without_panic() {
    assert_eq!(
        redact("/investigation run inv-1 --param label=café naïve --connection demo"),
        "/investigation run inv-1 --param label=… --connection demo"
    );
    assert_eq!(
        redact("/investigation run inv-1 --param=label=café naïve"),
        "/investigation run inv-1 --param=label=…"
    );
    assert_idempotent("/investigation run inv-1 --param label=café naïve --connection demo");
    assert_idempotent("/investigation run inv-1 --param=label=café naïve");
}

/// A quoted multiword value on the attached spelling redacts with the rest
/// of the region through the next exact run flag.
#[test]
fn an_attached_quoted_multiword_value_redacts_wholly() {
    let line = "/investigation run inv-1 --param=label=\"two words\" more --connection demo";
    assert_eq!(
        redact(line),
        "/investigation run inv-1 --param=label=… --connection demo"
    );
    assert_idempotent(line);
}

/// A trailing bare `--param` after a consumed binding: the closed zone is
/// not revisited and the bare flag keeps its malformed-entry shape.
#[test]
fn a_trailing_bare_param_after_a_binding_redacts() {
    let line = "/investigation run inv-1 --param label=hello --param";
    assert_eq!(
        redact(line),
        "/investigation run inv-1 --param label=… --param …"
    );
    assert_idempotent(line);
}

/// An attached form closes at the next exact `--param`: that flag is then
/// processed normally, and `--connection` after it passes through.
#[test]
fn an_attached_form_closes_at_the_next_exact_param() {
    let line = "/investigation run inv-1 --param=a=1 --param b=2 --connection demo";
    assert_eq!(
        redact(line),
        "/investigation run inv-1 --param=a=… --param b=… --connection demo"
    );
    assert_idempotent(line);
}

/// Consecutive attached forms: the region runs through them to the next
/// exact run flag — every attached binding after the first is consumed too.
#[test]
fn consecutive_attached_forms_redact_through_the_next_exact_flag() {
    let line = "/investigation run inv-1 --param=a=1 --param=b=2 --connection demo";
    let redacted = redact(line);
    assert_eq!(
        redacted,
        "/investigation run inv-1 --param=a=… --connection demo"
    );
    assert!(
        !redacted.contains("b=2"),
        "the later binding stays hidden: {redacted:?}"
    );
    assert_idempotent(line);
}

/// A valueless `--param` before another run flag: the zone's whitespace
/// survives so the next flag stays a separate token — otherwise the
/// redacted line would not be idempotent (`…` glued to the flag).
#[test]
fn a_bare_param_before_another_flag_stays_idempotent() {
    let line = "/investigation run inv-1 --param --connection demo";
    let once = redact(line);
    assert_eq!(once, "/investigation run inv-1 --param … --connection demo");
    assert_idempotent(line);
}

/// Totality (D8): over a generated corpus of investigation lines built from
/// a small token alphabet — the run flags, the attached spelling, and
/// value-shaped fragments, including non-ASCII and quote characters — the
/// walk never panics and is idempotent on every line. Deterministic: every
/// 4-token sequence is generated exactly once.
#[test]
fn the_walk_is_total_and_idempotent_over_a_generated_corpus() {
    const TOKENS: [&str; 9] = [
        "--param",
        "--param=",
        "=",
        "x",
        "é",
        "\"",
        "--connection",
        "--revalidate",
        "--param-spec",
    ];
    const BASE: u32 = TOKENS.len() as u32;
    let mut checked = 0;
    for i in 0..BASE.pow(4) {
        let mut rest = i;
        let mut tokens = Vec::with_capacity(4);
        for _ in 0..4 {
            tokens.push(TOKENS[(rest % BASE) as usize]);
            rest /= BASE;
        }
        let separator = if i % 7 == 0 { "  " } else { " " };
        let prefix = if i % 11 == 0 {
            "/investigations run x "
        } else if i % 17 == 0 {
            ""
        } else {
            "/investigation run x "
        };
        let line = format!("{prefix}{}{separator}", tokens.join(separator));
        let once = redact(&line);
        assert_eq!(
            redact(&once),
            once,
            "redaction must be idempotent on the generated line {line:?} -> {once:?}"
        );
        checked += 1;
    }
    assert!(
        checked >= 2000,
        "the generated corpus is large enough: {checked}"
    );
}
