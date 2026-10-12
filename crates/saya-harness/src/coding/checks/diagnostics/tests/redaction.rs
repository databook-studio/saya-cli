//! Redaction: parsed diagnostics expose counts and bounds only; child/rendered text never leaks.

use serde_json::json;

use super::{diagnostic, fixture_root, line, parse_at, span};
use saya_types::CheckDiagnosticFormat;

#[test]
fn parser_outputs_debug_only_counts_and_bounds() {
    let root = fixture_root();
    let secret = "diagnostic-message-sentinel";
    let parsed = parse_at(
        CheckDiagnosticFormat::RustcJson,
        &line(&diagnostic(
            secret,
            "error",
            json!({"code":"E0425"}),
            vec![span("private.rs", 1, 1, true)],
        )),
        root.as_path(),
    )
    .unwrap();
    let debug = format!(
        "{parsed:?} {:?} {:?}",
        parsed.entries()[0],
        parsed.entries()[0].locations()[0]
    );
    assert!(!debug.contains(secret));
    assert!(!debug.contains("private.rs"));
}

#[test]
fn discarded_child_and_rendered_text_never_reaches_the_parsed_set() {
    let root = fixture_root();
    let input = format!(
        "{}\n",
        line(&diagnostic(
            "unused variable",
            "warning",
            json!({"code": "unused_variables"}),
            vec![span("src/lib.rs", 4, 3, true)],
        )),
    );
    // Positive control: the sentinels are really present in what the tool printed.
    assert!(input.contains("child-secret-must-not-be-retained"));
    assert!(input.contains("rendered-secret-must-not-be-retained"));
    let parsed = parse_at(CheckDiagnosticFormat::RustcJson, &input, root.as_path()).unwrap();
    let mut retained = format!("{parsed:?}");
    for entry in parsed.entries() {
        retained.push_str(entry.message());
        retained.push_str(entry.code().unwrap_or(""));
        for location in entry.locations() {
            retained.push_str(location.path());
        }
    }
    assert!(
        !retained.contains("secret-must-not-be-retained"),
        "{retained}"
    );
}
