use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    parse::parse,
    types::{DiagnosticError, DiagnosticSeverity},
};
use saya_types::CheckDiagnosticFormat;

const RUSTC: CheckDiagnosticFormat = CheckDiagnosticFormat::RustcJson;
const CARGO: CheckDiagnosticFormat = CheckDiagnosticFormat::CargoJson;

#[cfg(windows)]
fn fixture_root() -> PathBuf {
    PathBuf::from("C:/saya-parser-fixture")
}

#[cfg(not(windows))]
fn fixture_root() -> PathBuf {
    PathBuf::from("/saya-parser-fixture")
}

fn span(file: &str, line: u64, column: u64, primary: bool) -> Value {
    json!({
        "file_name": file,
        "line_start": line,
        "line_end": line,
        "column_start": column,
        "column_end": column + 1,
        "is_primary": primary,
    })
}

fn diagnostic(message: &str, level: &str, code: Value, spans: Vec<Value>) -> Value {
    json!({
        "$message_type": "diagnostic",
        "message": message,
        "code": code,
        "level": level,
        "spans": spans,
        "children": [{"message": "child-secret-must-not-be-retained"}],
        "rendered": "rendered-secret-must-not-be-retained",
        "unknown_extra_field": true,
    })
}

fn line(value: &Value) -> String {
    serde_json::to_string(value).unwrap()
}

fn parse_at(
    format: CheckDiagnosticFormat,
    input: &str,
    root: &Path,
) -> Result<super::types::DiagnosticSet, DiagnosticError> {
    parse(format, input.as_bytes(), root)
}

macro_rules! rejects {
    ($format:expr, $input:expr, $root:expr, $error:expr) => {
        assert_eq!(parse($format, $input, $root).unwrap_err(), $error)
    };
}

macro_rules! rejects_text {
    ($format:expr, $input:expr, $root:expr, $error:expr) => {
        rejects!($format, $input.as_bytes(), $root, $error)
    };
}

macro_rules! rejects_value {
    ($format:expr, $value:expr, $root:expr, $error:expr) => {
        rejects_text!($format, &line($value), $root, $error)
    };
}

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

#[test]
fn malformed_json_utf8_and_all_bounds_refuse_partial_sets() {
    let root = fixture_root();
    rejects!(RUSTC, &[0xff], root.as_path(), DiagnosticError::InvalidUtf8);
    rejects_text!(
        RUSTC,
        "{bad",
        root.as_path(),
        DiagnosticError::InvalidRecord { line: 1 }
    );
    rejects!(
        RUSTC,
        &vec![b' '; 65_537],
        root.as_path(),
        DiagnosticError::InputTooLarge
    );
    rejects_text!(
        RUSTC,
        &format!("{}\n", " ".repeat(32_769)),
        root.as_path(),
        DiagnosticError::RecordTooLarge { line: 1 }
    );
    let too_many = (0..129)
        .map(|_| line(&diagnostic("x", "note", Value::Null, Vec::new())))
        .collect::<Vec<_>>()
        .join("\n");
    rejects_text!(
        RUSTC,
        &too_many,
        root.as_path(),
        DiagnosticError::TooManyDiagnostics
    );
    let invalid = DiagnosticError::InvalidDiagnostic { line: 1 };
    rejects_value!(
        RUSTC,
        &diagnostic(&"x".repeat(4097), "note", Value::Null, Vec::new()),
        root.as_path(),
        invalid.clone()
    );
    rejects_value!(
        RUSTC,
        &diagnostic("x", "note", json!({"code": "x".repeat(65)}), Vec::new()),
        root.as_path(),
        invalid.clone()
    );
    for spans in [
        (0..129).map(|_| span("a.rs", 1, 1, false)).collect(),
        (0..9).map(|_| span("a.rs", 1, 1, true)).collect(),
    ] {
        rejects_value!(
            RUSTC,
            &diagnostic("x", "note", Value::Null, spans),
            root.as_path(),
            invalid.clone()
        );
    }
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
