//! Input limits: malformed JSON/UTF-8 and every documented bound refuse partial sets.

use serde_json::{Value, json};

use super::{
    DiagnosticError, RUSTC, diagnostic, fixture_root, line, parse, rejects, rejects_text,
    rejects_value, span,
};

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
