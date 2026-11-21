//! The one-shot warning is terse by design; doctor is where a user learns
//! *which* setup detail is stale and what to write instead. An old tool
//! spelling in `[tools] allow` is exactly that: honoured, never an error, and
//! worth getting rid of.

use super::tools_deprecation_lines;

/// A stated old spelling is reported with the word to use instead — the
/// spelling, its successor, and the release it stops working in.
#[test]
fn doctor_names_each_deprecated_tool_spelling_and_its_successor() {
    let notes = vec![
        "`run_command` is a deprecated spelling of `shell`".to_string(),
        "`http_download` is a deprecated spelling of `web_fetch`".to_string(),
    ];
    let report = tools_deprecation_lines(&notes).join("\n");
    assert!(
        report.contains("run_command") && report.contains("shell"),
        "doctor must name the spelling and its successor: {report}"
    );
    assert!(
        report.contains("http_download") && report.contains("web_fetch"),
        "doctor must name every note it was given: {report}"
    );
    assert!(
        report.contains("[tools] allow"),
        "doctor must say which setting the note is about: {report}"
    );
}

/// A canonical list adds no line: doctor is silent when nothing is stale.
#[test]
fn doctor_is_silent_when_no_spelling_is_deprecated() {
    assert!(
        tools_deprecation_lines(&[]).is_empty(),
        "no deprecated spelling means no section"
    );
}
