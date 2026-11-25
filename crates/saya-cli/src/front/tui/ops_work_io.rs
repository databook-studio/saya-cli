//! The two gestures every shared read shares: a panel section built from the
//! session's own facts, and one printed operation run through the crate's
//! capture seam and filed as the entry it printed.

use super::ops::{Asked, OpsDeps};
use crate::chat::ops_status::{Note, Section, StatusPanel};

/// What `/explain` says with no statement to explain.
pub(super) const EXPLAIN_NEEDS_A_STATEMENT: &str =
    "`/explain` needs a statement: `/explain <sql>` explains the one you name";

/// One bounded, read-only statement through the shared session path, with what
/// it printed as the entry.
pub(super) fn statement(deps: &OpsDeps, sql: String) -> Asked {
    let runtime = deps.runtime.current();
    let profile = deps.state().profile.clone();
    captured(
        crate::interactive::session_sql::run(
            &runtime,
            profile.as_deref(),
            &sql,
            false,
            deps.format,
        ),
        |_| true,
    )
}

/// The panel's one section, plus the alias' note, exactly as `/workspace`
/// and `/context` answer on the line front.
pub(super) fn section_answer(deps: &OpsDeps, section: Section) -> String {
    let runtime = deps.runtime.current();
    let mut state = deps.state();
    if let Section::Instructions = section {
        return crate::chat::ops_status::instructions_answer(
            &mut state,
            &runtime,
            deps.root.as_deref(),
        );
    }
    let text = StatusPanel::build(
        &crate::chat::ops_status::StatusFacts::of_session(&state).with_runtime(
            &state,
            &runtime,
            &deps.store,
            deps.root.as_deref(),
        ),
    )
    .render_section(section);
    match section {
        Section::Workspace => crate::chat::ops_status::with_note(&mut state, Note::Workspace, text),
        _ => text,
    }
}

/// Runs one printed operation with the crate's capture seam, and answers with
/// what it printed: the notice on success, the error otherwise.
pub(super) fn captured<T>(
    outcome: impl std::future::Future<Output = Result<T, Box<dyn std::error::Error>>>,
    succeeded: impl FnOnce(T) -> bool,
) -> Asked {
    crate::commands::capture_output_start();
    let result = block_on(outcome);
    let (out, err) = crate::commands::capture_output_take();
    let body = if out.trim().is_empty() { err } else { out };
    let body = body.trim_end().to_owned();
    match result {
        Ok(value) => {
            if succeeded(value) {
                if body.is_empty() {
                    Asked::Notice(String::new())
                } else {
                    Asked::Notice(body)
                }
            } else {
                Asked::Error(body)
            }
        }
        Err(error) => Asked::Error(error.to_string()),
    }
}

/// The one blocking seam the crate uses for a shared operation inside a
/// synchronous surface: the session's own.
pub(super) fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    crate::interactive::session::lifecycle::resume::block_on(future)
}
