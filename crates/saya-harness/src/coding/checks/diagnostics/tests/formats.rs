//! Format acceptance: rustc/cargo record shapes, the documented severity table, and grammar mismatches.

use serde_json::{Value, json};

use super::{
    CARGO, DiagnosticError, DiagnosticSeverity, RUSTC, diagnostic, fixture_root, line, parse,
    parse_at, rejects, rejects_text, rejects_value, span,
};
use saya_types::CheckDiagnosticFormat;

#[test]
fn each_explicit_format_parses_only_its_diagnostic_records() {
    let root = fixture_root();
    let direct = format!(
        "{}\n{}\n",
        line(&json!({"$message_type": "artifact", "artifact": "libdemo.rlib", "emit": "link"})),
        line(&diagnostic(
            "unused variable",
            "warning",
            json!({"code": "unused_variables", "extra": null}),
            vec![span("src/lib.rs", 4, 3, true)],
        )),
    );
    let parsed = parse_at(CheckDiagnosticFormat::RustcJson, &direct, root.as_path()).unwrap();
    assert_eq!(parsed.parser_revision(), 1);
    assert_eq!(parsed.entries().len(), 1);
    assert_eq!(parsed.entries()[0].severity(), DiagnosticSeverity::Warning);
    assert_eq!(parsed.entries()[0].code(), Some("unused_variables"));
    assert_eq!(parsed.entries()[0].message(), "unused variable");
    assert_eq!(parsed.entries()[0].locations()[0].path(), "src/lib.rs");

    let wrapped = format!(
        "{}\n{}\n{}\n{}\n",
        line(
            &json!({"reason": "compiler-artifact", "package_id": "demo 0.1.0", "target": {}, "profile": {}, "filenames": [], "fresh": true, "extra": "ignored"})
        ),
        line(
            &json!({"reason": "build-script-executed", "package_id": "demo 0.1.0", "linked_libs": [], "cfgs": [], "env": [], "out_dir": "target/out"})
        ),
        line(&json!({
            "reason": "compiler-message",
            "message": diagnostic("missing item", "error", json!({"code": "E0425"}),
                vec![span("src/main.rs", 8, 2, true)]),
            "extra": "tolerated"
        })),
        line(&json!({"reason": "build-finished", "success": false})),
    );
    let parsed = parse_at(CheckDiagnosticFormat::CargoJson, &wrapped, root.as_path()).unwrap();
    assert_eq!(parsed.entries().len(), 1);
    assert_eq!(parsed.entries()[0].severity(), DiagnosticSeverity::Error);
    assert_eq!(parsed.entries()[0].code(), Some("E0425"));
}

#[test]
fn documented_severities_and_empty_stream_are_explicit() {
    let root = fixture_root();
    let levels = [
        ("error", DiagnosticSeverity::Error),
        ("warning", DiagnosticSeverity::Warning),
        ("note", DiagnosticSeverity::Note),
        ("help", DiagnosticSeverity::Help),
        ("failure-note", DiagnosticSeverity::FailureNote),
        (
            "error: internal compiler error",
            DiagnosticSeverity::InternalCompilerError,
        ),
    ];
    for (level, expected) in levels {
        let parsed = parse_at(
            CheckDiagnosticFormat::RustcJson,
            &line(&diagnostic("x", level, Value::Null, Vec::new())),
            root.as_path(),
        )
        .unwrap();
        assert_eq!(parsed.entries()[0].severity(), expected);
    }
    assert!(
        parse_at(CheckDiagnosticFormat::RustcJson, "", root.as_path())
            .unwrap()
            .entries()
            .is_empty()
    );
}

#[test]
fn grammar_mismatch_unknown_records_and_levels_are_incomplete() {
    let root = fixture_root();
    let direct = line(&diagnostic("x", "error", Value::Null, Vec::new()));
    let cargo = line(&json!({"reason": "compiler-message", "message": diagnostic(
        "x", "error", Value::Null, Vec::new())}));
    let unsupported = DiagnosticError::UnsupportedFormat;
    rejects_text!(CARGO, &direct, root.as_path(), unsupported.clone());
    rejects_text!(RUSTC, &cargo, root.as_path(), unsupported);
    rejects_value!(
        CARGO,
        &json!({"reason": "future-event"}),
        root.as_path(),
        DiagnosticError::InvalidRecord { line: 1 }
    );
    rejects_value!(
        RUSTC,
        &diagnostic("x", "mystery", Value::Null, Vec::new()),
        root.as_path(),
        DiagnosticError::InvalidDiagnostic { line: 1 }
    );
    rejects_value!(
        RUSTC,
        &json!({"$message_type": "future_incompat"}),
        root.as_path(),
        DiagnosticError::UnsupportedFormat
    );
}
