//! Status-bar tests: the approval colour map pins the mode grammar, and the
//! action/draft wording pins Phase 4. Beside the renderer per the testing
//! standard (the `view.rs`, `scroll.rs`, `chapters.rs`, `tool_buffer.rs`
//! pattern), so `status.rs` stays under the 250-line hard cap.

use super::super::theme::{danger, secondary, success, warning};
use super::{approval_colour, status_bg, status_spans};
use crate::interactive::session_prompt::StatusView;

fn bypass_view() -> StatusView {
    StatusView {
        profile: "analytics".into(),
        included: Vec::new(),
        provider: "ollama".into(),
        model: "m".into(),
        approval_mode: "bypass".into(),
        agent_mode: "build".into(),
        workspace_root: None,
        sharing_on: false,
        host_composed: false,
        denied_programs: Vec::new(),
        task_summary: None,
    }
}

/// The colour map carries an explicit arm for every mode the grammar
/// parses; the catch-all (`secondary()`) is the quiet-drift hole a fourth
/// variant would fall into. `bypass` renders `danger()` red: the mode
/// that claims "everything runs" must read as the danger it is, in the
/// grammar's own word, on every surface.
#[test]
fn the_status_colour_map_has_an_arm_for_every_mode() {
    for (mode, expected) in [
        ("read-only", success()),
        ("ask", warning()),
        ("never", danger()),
        ("bypass", danger()),
    ] {
        assert_eq!(
            approval_colour(mode),
            expected,
            "{mode} must have its own colour arm"
        );
    }
    assert_eq!(
        approval_colour("whatever-a-future-parse-site-forgot"),
        secondary(),
        "the catch-all is named, not removed: unknown words stay grey"
    );
}

/// The two status surfaces agree on the bypass mode: the headless
/// one-line header renders `approval:bypass`, and the TUI bar renders the
/// same word in the same `danger()` red — the parity the
/// `status_segments_mirror_status_line_polarity` precedent pins for
/// sharing, here for the mode the red indicator belongs to.
#[test]
fn the_status_surfaces_render_approval_colon_bypass_in_danger_colour() {
    let mut state = crate::interactive::session_state::SessionState::new(
        "s1",
        Some(String::from("analytics")),
        "m",
    );
    state.approval_mode = "bypass".into();
    let headless = crate::interactive::session_prompt::status_line(&state);
    assert!(
        headless.contains("approval:bypass"),
        "the headless status line says approval:bypass: {headless}"
    );

    let spans = status_spans(&bypass_view(), status_bg());
    let approval = spans
        .iter()
        .find(|span| span.content.starts_with("approval:"))
        .expect("the status bar carries an approval segment");
    assert_eq!(
        approval.content.as_ref(),
        "approval:bypass ",
        "the TUI bar says the same words as the headless line"
    );
    assert_eq!(
        approval.style.fg,
        Some(danger()),
        "bypass renders in danger red, never a softening colour"
    );
}

/// The TUI bar carries the `mode:` segment beside `approval:`, with the
/// same words the headless header renders — the anti-drift check for the
/// posture `/mode` switches.
#[test]
fn the_status_bar_carries_the_mode_segment() {
    let spans = status_spans(&bypass_view(), status_bg());
    let mode = spans
        .iter()
        .find(|span| span.content.starts_with("mode:"))
        .expect("the status bar carries a mode segment");
    assert_eq!(
        mode.content.as_ref(),
        "mode:build ",
        "the TUI bar says the same words as the headless line"
    );
}
