//! Location handling: primary-span sorting, candidate containment, and location bounds.

use serde_json::{Value, json};

use super::{
    DiagnosticError, RUSTC, diagnostic, fixture_root, line, parse, parse_at, rejects, rejects_text,
    rejects_value, span,
};
use saya_types::CheckDiagnosticFormat;

#[test]
fn primary_locations_are_all_sorted_and_global_diagnostics_are_valid() {
    let root = fixture_root();
    let input = format!(
        "{}\n{}",
        line(&diagnostic(
            "multi-span",
            "note",
            Value::Null,
            vec![
                span("z.rs", 3, 4, true),
                span("context.rs", 1, 1, false),
                span("a.rs", 2, 5, true),
                span("z.rs", 3, 4, true),
            ],
        )),
        line(&diagnostic("global", "help", Value::Null, Vec::new())),
    );
    let parsed = parse_at(CheckDiagnosticFormat::RustcJson, &input, root.as_path()).unwrap();
    assert_eq!(parsed.entries().len(), 2);
    let locations = parsed.entries()[0].locations();
    assert_eq!(locations.len(), 2);
    assert_eq!(locations[0].path(), "a.rs");
    assert_eq!(locations[1].path(), "z.rs");
    assert!(parsed.entries()[1].locations().is_empty());
    rejects_value!(
        RUSTC,
        &diagnostic(
            "context only",
            "note",
            Value::Null,
            vec![span("context.rs", 1, 1, false)]
        ),
        root.as_path(),
        DiagnosticError::InvalidLocation { line: 1 }
    );
}

#[test]
fn location_bounds_and_candidate_containment_are_enforced() {
    let root = fixture_root();
    for path in [
        "../escape.rs",
        "/outside/escape.rs",
        "src//odd.rs",
        "src/./odd.rs",
    ] {
        rejects_value!(
            RUSTC,
            &diagnostic("x", "error", Value::Null, vec![span(path, 1, 1, true)]),
            root.as_path(),
            DiagnosticError::InvalidLocation { line: 1 }
        );
    }
    let long_path = "a".repeat(513);
    let invalid = DiagnosticError::InvalidLocation { line: 1 };
    rejects_value!(
        RUSTC,
        &diagnostic(
            "x",
            "error",
            Value::Null,
            vec![span(&long_path, 1, 1, true)]
        ),
        root.as_path(),
        invalid.clone()
    );
    rejects_value!(
        RUSTC,
        &diagnostic(
            "x",
            "error",
            Value::Null,
            vec![span("src/line\nfeed.rs", 1, 1, true)]
        ),
        root.as_path(),
        invalid.clone()
    );
    let absolute = format!("{}/src/main.rs", root.to_str().unwrap());
    let valid = line(&diagnostic(
        "x",
        "error",
        Value::Null,
        vec![span(&absolute, 1, 1, true)],
    ));
    assert_eq!(
        parse_at(CheckDiagnosticFormat::RustcJson, &valid, root.as_path())
            .unwrap()
            .entries()[0]
            .locations()[0]
            .path(),
        "src/main.rs"
    );
    for (line_start, line_end, column_start, column_end) in [
        (0, 1, 1, 1),
        (1, 10_000_001, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 1_000_001, 1_000_001),
        (2, 1, 1, 1),
        (1, 1, 2, 1),
    ] {
        let bad_span = span("src/main.rs", line_start, column_start, true);
        let mut bad_span = bad_span;
        bad_span["line_end"] = json!(line_end);
        bad_span["column_end"] = json!(column_end);
        rejects_value!(
            RUSTC,
            &diagnostic("x", "error", Value::Null, vec![bad_span]),
            root.as_path(),
            invalid.clone()
        );
    }
}
