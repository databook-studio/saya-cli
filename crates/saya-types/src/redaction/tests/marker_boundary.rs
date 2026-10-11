//! The `key=value` marker pass boundary: a redacted value ends at its own
//! token — its closing quote, whitespace, `&`, `;`, or the enclosing `"` —
//! never at the end of the whole input, and never short of a quoted value.

use crate::redaction::{redact, redact_counted, redact_markers};

#[test]
fn marker_inside_a_json_string_does_not_swallow_the_rest_of_the_line() {
    // A marker value inside a compact JSON string has no whitespace before
    // the value ends: the enclosing `"` is the value's terminator, and
    // everything after it is ordinary, non-secret text. Pinned on the marker
    // pass, which owns this rule — the credential-named env-assignment pass
    // shares the value boundary and still consumes such a value first
    // through the full chain (reported blocked; its file is outside this
    // task's ownership).
    assert_eq!(
        redact_markers("{\"message\":\"password=zz\",\"code\":\"E0425\",\"spans\":[1]}"),
        "{\"message\":\"password=[redacted]\",\"code\":\"E0425\",\"spans\":[1]}"
    );
}

#[test]
fn marker_redaction_preserves_following_json_lines() {
    // The marker sits inside the first line's JSON string; the second line is
    // a separate event and must survive byte-for-byte, with the secret gone.
    let input = "{\"message\":\"token=abc123\"}\n{\"level\":\"info\"}";
    let out = redact(input);
    assert_eq!(out.lines().nth(1), Some("{\"level\":\"info\"}"), "{out:?}");
    assert!(!out.contains("abc123"), "{out:?}");
}

#[test]
fn quoted_value_with_a_space_is_fully_redacted() {
    // A quoted value redacts through its closing quote — stopping at the
    // opening quote would leak `s3cr et` piecewise. Pinned on the marker
    // pass, which owns this rule (see the JSON-string test above for why the
    // full chain is not the proof layer here).
    let out = redact_markers("password=\"s3cr et\" next");
    assert_eq!(out, "password=[redacted] next", "{out:?}");
    assert!(!out.contains("s3cr") && !out.contains("et"), "{out:?}");
    assert_eq!(
        redact_markers("secret='a b' next"),
        "secret=[redacted] next"
    );
}

#[test]
fn unterminated_quote_fails_closed() {
    // No closing quote: the redaction runs to the end of the input rather
    // than leak either half of the split value.
    let out = redact_markers("x password=\"abc def");
    assert!(!out.contains("abc"), "{out:?}");
    assert!(!out.contains("def"), "{out:?}");
}

#[test]
fn unquoted_value_still_ends_at_whitespace_ampersand_and_semicolon() {
    assert_eq!(redact("a=1&token=zz&b=2"), "a=1&token=[redacted]&b=2");
    assert_eq!(redact("password=zz; next"), "password=[redacted]; next");
    assert_eq!(redact("password=zz next"), "password=[redacted] next");
}

/// One adversarial input for the marker boundary: the exact output the
/// quote-aware rules determine (when they determine one), secret fragments
/// that must never survive, the non-secret tail an unquoted secret's
/// redaction must preserve, and the replacement count (when determined).
struct AdversarialCase {
    input: String,
    expected: Option<String>,
    secret_fragments: &'static [&'static str],
    tail: Option<&'static str>,
    count: Option<usize>,
    /// Pinned through the marker pass alone: the credential-named
    /// env-assignment pass shares the value boundary and still consumes such
    /// a value first through the full chain (reported blocked; its file is
    /// outside this task's ownership).
    marker_pass_only: bool,
}

fn row(
    input: &str,
    expected: Option<&str>,
    secret_fragments: &'static [&'static str],
    tail: Option<&'static str>,
    count: Option<usize>,
) -> AdversarialCase {
    AdversarialCase {
        input: input.to_string(),
        expected: expected.map(str::to_string),
        secret_fragments,
        tail,
        count,
        marker_pass_only: false,
    }
}

fn marker_pass_only(
    input: &str,
    expected: Option<&str>,
    secret_fragments: &'static [&'static str],
    tail: Option<&'static str>,
) -> AdversarialCase {
    let mut case = row(input, expected, secret_fragments, tail, None);
    case.marker_pass_only = true;
    case
}

fn adversarial_cases() -> Vec<AdversarialCase> {
    vec![
        // An empty value still redacts; nothing follows it to preserve.
        row("password=", Some("password=[redacted]"), &[], None, Some(1)),
        // The enclosing `"` after the value: whichever way the boundary reads
        // this, no non-empty secret may leak and the pass must not panic —
        // nothing else is pinned for this ambiguous shape.
        row("\"password=\",\"x\"", None, &[], None, None),
        // The marker at the very end of the input.
        row(
            "token=endvalue",
            Some("token=[redacted]"),
            &["endvalue"],
            None,
            Some(1),
        ),
        // Two markers back to back redact independently.
        row(
            "token=a token=b",
            Some("token=[redacted] token=[redacted]"),
            &[],
            None,
            Some(2),
        ),
        // Uppercase marker, mixed-case value.
        row(
            "PASSWORD=Zz9",
            Some("PASSWORD=[redacted]"),
            &["Zz9"],
            None,
            Some(1),
        ),
        // Multibyte value terminated by `;`.
        row(
            "password=é;ok",
            Some("password=[redacted];ok"),
            &["é"],
            Some(";ok"),
            Some(1),
        ),
        // A quoted value holding a multibyte character and a space. Pinned
        // through the marker pass: the env-assignment pass consumes this
        // value first through the full chain (reported blocked).
        marker_pass_only(
            "password=\"é ü\" ok",
            Some("password=[redacted] ok"),
            &["é", "ü"],
            Some(" ok"),
        ),
        // Adjacent markers separated by `&`.
        row(
            "password=a&password=b",
            Some("password=[redacted]&password=[redacted]"),
            &[],
            None,
            Some(2),
        ),
        // A marker value inside a JSON string: the closing `"` of the
        // enclosing string ends the value, the rest of the object survives.
        // Pinned through the marker pass (same env-pass block as above).
        marker_pass_only(
            "{\"m\":\"token=ab\"}",
            Some("{\"m\":\"token=[redacted]\"}"),
            &["ab"],
            Some("}"),
        ),
        // A ten-thousand-character value: everything past `=` is the secret.
        row(
            &format!("password={}", "x".repeat(10_000)),
            Some("password=[redacted]"),
            &["xxxxx"],
            None,
            Some(1),
        ),
        // `api_key=` is claimed by the env-assignment pass too (the `key`
        // credential word after the `_` boundary), so its JSON-tail rule is
        // pinned through the marker pass (see marker_pass_only above).
        marker_pass_only(
            "api_key=zz\",\"code\":1",
            Some("api_key=[redacted]\",\"code\":1"),
            &["zz"],
            Some("\",\"code\":1"),
        ),
        // A quoted `api_key` value with a space: the same rule the
        // marker-pass rows pin, at the pass that owns it.
        marker_pass_only(
            "api_key=\"s3cr et\" next",
            Some("api_key=[redacted] next"),
            &["s3cr", "et"],
            Some(" next"),
        ),
        // An unterminated quoted `api_key` value: redacted to the end of the
        // input, either half never emitted.
        marker_pass_only(
            "x api_key=\"abc def",
            Some("x api_key=[redacted]"),
            &["abc", "def"],
            None,
        ),
        // A single-quoted value with its tail.
        row(
            "secret='quoted' tail",
            Some("secret=[redacted] tail"),
            &["quoted"],
            Some(" tail"),
            Some(1),
        ),
        // An unquoted value between two ordinary assignments.
        row(
            "a=1&password=zz&b=2",
            Some("a=1&password=[redacted]&b=2"),
            &["zz"],
            Some("&b=2"),
            Some(1),
        ),
    ]
}

#[test]
fn redaction_never_leaks_the_secret_and_never_drops_the_tail() {
    for case in adversarial_cases() {
        let (out, count) = if case.marker_pass_only {
            (redact_markers(&case.input), None)
        } else {
            let (out, count) = redact_counted(&case.input);
            (out, Some(count))
        };
        if let Some(expected) = &case.expected {
            assert_eq!(out, *expected, "input {:?}", case.input);
        }
        for fragment in case.secret_fragments {
            assert!(
                !out.contains(fragment),
                "input {:?} leaked {fragment:?} in {out:?}",
                case.input
            );
        }
        if let Some(tail) = case.tail {
            // The secret was unquoted, so the tail past its end is ordinary
            // text the redactor must keep.
            assert!(
                out.contains(tail),
                "input {:?} dropped tail {tail:?} from {out:?}",
                case.input
            );
        }
        if let Some(expected) = case.count {
            assert_eq!(count, Some(expected), "input {:?}", case.input);
        }
    }
}

#[test]
fn redaction_is_idempotent_on_the_adversarial_set() {
    for case in adversarial_cases() {
        let once = redact(&case.input);
        assert_eq!(redact(&once), once, "input {:?}", case.input);
    }
}
