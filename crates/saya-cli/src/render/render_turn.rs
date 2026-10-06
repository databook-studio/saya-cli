//! The one terminal result a scripted run reports in text mode: nothing for
//! the result itself — except the unfinished-tools warning, one stderr line
//! naming the calls whose effects the stream never settled (a cancelled
//! `workspace_edit` may still have written the file). Under json/ndjson the
//! machine-readable `turn_result` event ends the stdout stream.

use crate::render::{RenderFormat, Rendered, TerminalEvent, TurnStatus, render_event};

/// The text-mode warning for calls whose effects are unknown: one line, the
/// entries joined by `; `, carrying names and paths only — never file
/// contents. `None` when every call settled.
pub(crate) fn unfinished_warning(entries: &[String]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    Some(format!(
        "Unfinished tool calls (their effects are unknown — check these files): {}",
        entries.join("; ")
    ))
}

/// The one terminal result a scripted run emits: under json/ndjson the
/// machine-readable `turn_result` event on stdout — the LAST stdout line of
/// the run, its `exit_code` equal to the process exit code; in text mode
/// only the unfinished-tools warning on stderr, and only when there is one.
pub(crate) fn turn_result_rendered(
    format: RenderFormat,
    status: TurnStatus,
    exit_code: i32,
    tools_completed: usize,
    tools_unfinished: &[String],
) -> Rendered {
    match format {
        RenderFormat::Text => match unfinished_warning(tools_unfinished) {
            Some(message) => render_event(&TerminalEvent::Diagnostic { message }, format),
            None => Rendered {
                stdout: String::new(),
                stderr: String::new(),
            },
        },
        RenderFormat::Json | RenderFormat::Ndjson => render_event(
            &TerminalEvent::TurnResult {
                status,
                exit_code,
                tools_completed,
                tools_unfinished: tools_unfinished.to_vec(),
            },
            format,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn the_text_warning_exists_only_when_calls_are_unfinished() {
        assert!(unfinished_warning(&[]).is_none());
        let warning = unfinished_warning(&entries(&[
            "workspace_edit src/lib.rs",
            "workspace_write notes/a.md",
        ]))
        .expect("a warning exists for unfinished calls");
        assert_eq!(
            warning,
            "Unfinished tool calls (their effects are unknown — check these files): \
             workspace_edit src/lib.rs; workspace_write notes/a.md"
        );
    }

    /// Text mode says nothing for a settled turn, and exactly the one
    /// diagnostic line — on stderr, never stdout — when a call's effect is
    /// unknown.
    #[test]
    fn text_mode_says_nothing_unless_the_warning_exists() {
        let rendered = turn_result_rendered(RenderFormat::Text, TurnStatus::Cancelled, 5, 1, &[]);
        assert_eq!(rendered.stdout, "");
        assert_eq!(rendered.stderr, "");
        let rendered = turn_result_rendered(
            RenderFormat::Text,
            TurnStatus::Cancelled,
            5,
            1,
            &entries(&["workspace_write notes/after.txt"]),
        );
        assert_eq!(rendered.stdout, "");
        assert_eq!(
            rendered.stderr,
            "Unfinished tool calls (their effects are unknown — check these files): \
             workspace_write notes/after.txt\n"
        );
    }

    #[test]
    fn json_mode_ends_the_stream_with_the_machine_readable_result() {
        let rendered = turn_result_rendered(RenderFormat::Ndjson, TurnStatus::Completed, 0, 2, &[]);
        assert_eq!(
            rendered.stdout,
            "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}\n"
        );
        assert_eq!(rendered.stderr, "");
    }
}
