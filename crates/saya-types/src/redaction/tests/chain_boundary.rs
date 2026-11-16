//! The bounded-value rule through the public `redact` entry point: every
//! credential-named assignment and `key=value` marker — whichever pass claims
//! the value first — ends its redaction at the value's own token, never at
//! the end of the whole input, and never short of a quoted value.

use crate::redaction::{redact, redact_counted};

#[test]
fn redact_keeps_the_line_after_a_json_string_marker() {
    // A marker value inside a compact JSON string has no whitespace before
    // the value ends: the enclosing `"` is the value's terminator, and
    // everything after it is ordinary, non-secret text.
    assert_eq!(
        redact("{\"message\":\"password=zz\",\"code\":\"E0425\",\"spans\":[1]}"),
        "{\"message\":\"password=[redacted]\",\"code\":\"E0425\",\"spans\":[1]}"
    );
}

#[test]
fn redact_preserves_following_json_lines() {
    // The marker sits inside the first line's JSON string; the second line is
    // a separate event and must survive byte-for-byte, with the secret gone.
    let input = "{\"message\":\"token=abc123\"}\n{\"level\":\"info\"}";
    let out = redact(input);
    assert_eq!(out.lines().nth(1), Some("{\"level\":\"info\"}"), "{out:?}");
    assert!(!out.contains("abc123"), "{out:?}");
}

#[test]
fn redact_fully_redacts_a_quoted_value_with_a_space() {
    // A quoted value redacts through its closing quote — stopping at the
    // opening quote would leak `s3cr et` piecewise.
    let out = redact("password=\"s3cr et\" next");
    assert_eq!(out, "password=[redacted] next", "{out:?}");
    assert!(!out.contains("s3cr") && !out.contains("et"), "{out:?}");
    assert_eq!(redact("secret='a b' next"), "secret=[redacted] next");
}

#[test]
fn redact_fails_closed_on_an_unterminated_quote() {
    // No closing quote: the redaction runs to the end of the input rather
    // than leak either half of the split value.
    let out = redact("x password=\"abc def");
    assert!(!out.contains("abc"), "{out:?}");
    assert!(!out.contains("def"), "{out:?}");
}

#[test]
fn redact_unquoted_values_still_end_at_whitespace_ampersand_and_semicolon() {
    assert_eq!(redact("a=1&token=zz&b=2"), "a=1&token=[redacted]&b=2");
    assert_eq!(redact("password=zz; next"), "password=[redacted]; next");
    assert_eq!(redact("password=zz next"), "password=[redacted] next");
}

/// One adversarial input through the public chain: the exact output the
/// bounded-value rules determine (when they determine one), secret fragments
/// that must never survive, the non-secret tail an unquoted secret's
/// redaction must preserve, and the replacement count (when determined).
struct AdversarialCase {
    input: String,
    expected: Option<String>,
    secret_fragments: &'static [&'static str],
    tail: Option<&'static str>,
    count: Option<usize>,
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
    }
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
        // A quoted value holding a multibyte character and a space.
        row(
            "password=\"é ü\" ok",
            Some("password=[redacted] ok"),
            &["é", "ü"],
            Some(" ok"),
            Some(1),
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
        row(
            "{\"m\":\"token=ab\"}",
            Some("{\"m\":\"token=[redacted]\"}"),
            &["ab"],
            Some("}"),
            Some(1),
        ),
        // A ten-thousand-character value: everything past `=` is the secret.
        row(
            &format!("password={}", "x".repeat(10_000)),
            Some("password=[redacted]"),
            &["xxxxx"],
            None,
            Some(1),
        ),
        // `api_key` is claimed by the env-assignment pass through the `key`
        // credential word after the `_` boundary: the value ends at the
        // enclosing `"`, the rest of the object survives.
        row(
            "api_key=zz\",\"code\":1",
            Some("api_key=[redacted]\",\"code\":1"),
            &["zz"],
            Some("\",\"code\":1"),
            Some(1),
        ),
        // The run's own credential namespace, with a quoted value holding a
        // space: redacted through the closing quote, tail preserved.
        row(
            "SAYA_RUN_EP_ROLE=\"a b\"",
            Some("SAYA_RUN_EP_ROLE=[redacted]"),
            &["a b"],
            None,
            Some(1),
        ),
        // A quoted `api_key` value with a space.
        row(
            "api_key=\"s3cr et\" next",
            Some("api_key=[redacted] next"),
            &["s3cr", "et"],
            Some(" next"),
            Some(1),
        ),
        // An unterminated quoted `api_key` value: redacted to the end of the
        // input, either half never emitted.
        row(
            "x api_key=\"abc def",
            Some("x api_key=[redacted]"),
            &["abc", "def"],
            None,
            Some(1),
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
fn redact_never_leaks_the_secret_and_never_drops_the_tail_on_the_adversarial_set() {
    for case in adversarial_cases() {
        let (out, count) = redact_counted(&case.input);
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
            assert_eq!(count, expected, "input {:?}", case.input);
        }
    }
}

#[test]
fn redact_is_idempotent_on_the_adversarial_set() {
    for case in adversarial_cases() {
        let once = redact(&case.input);
        assert_eq!(redact(&once), once, "input {:?}", case.input);
    }
}
