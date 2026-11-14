use super::*;

/// The mascot is the one tiny glyph the owner can redraw: four rows at most,
/// ten columns at most, every row the same width so centring does not skew
/// it, and nothing but block glyphs and spaces.
#[test]
fn mascot_fits_the_four_by_ten_budget() {
    assert!(MASCOT.len() <= 4, "at most four rows");
    let width = MASCOT[0].chars().count();
    assert!(width <= 10, "at most ten columns");
    for row in MASCOT {
        assert_eq!(row.chars().count(), width, "ragged row: {row:?}");
        for ch in row.chars() {
            assert!(
                matches!(
                    ch,
                    ' ' | '\u{2588}' | '\u{2580}' | '\u{2584}' | '\u{258c}' | '\u{2590}'
                ),
                "only block glyphs and spaces: {ch:?}"
            );
        }
    }
}

/// The figure is a right-facing profile: two tall ears on the top row, a
/// head with one eye gap and a snout that reaches furthest right, a body,
/// and a tail sweeping left along the ground.
#[test]
fn mascot_is_a_right_facing_profile() {
    let runs = |row: &str| row.trim().split(' ').filter(|run| !run.is_empty()).count();
    assert_eq!(runs(MASCOT[0]), 2, "two tall ears on the top row");
    assert_eq!(runs(MASCOT[1]), 2, "the eye is a gap in the head row");

    let right_edge = |row: &str| row.trim_end().chars().count();
    let snout = right_edge(MASCOT[1]);
    for (row, text) in MASCOT.iter().enumerate().skip(2) {
        assert!(
            snout > right_edge(text),
            "the snout leads on row {row}: {text:?}"
        );
    }

    let tail = MASCOT[MASCOT.len() - 1];
    assert!(
        tail.starts_with('\u{2584}'),
        "the tail sweeps left to the ground: {tail:?}"
    );
    assert!(
        MASCOT[..MASCOT.len() - 1]
            .iter()
            .any(|row| !row.trim().is_empty()),
        "a body sits above the tail"
    );
}

/// Every glyph is painted in the theme accent, and no glyph is dropped: the
/// figure must survive styling unchanged.
#[test]
fn the_mascot_is_painted_in_the_accent() {
    let lines = mascot_lines();
    assert_eq!(lines.len(), MASCOT.len());
    for (row, line) in MASCOT.iter().zip(lines) {
        let rendered: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(&rendered, row);
        for span in &line.spans {
            assert_eq!(span.style.fg, Some(accent()));
        }
    }
}

/// The status line names the model, the provider, and the permission mode
/// in that order; with no model it says the AI is not set up.
#[test]
fn status_line_names_model_provider_and_mode() {
    assert_eq!(
        status_text("qwen", "ollama", "read-only"),
        "qwen · ollama · read-only"
    );
    assert_eq!(status_text("", "ollama", "ask"), AI_NOT_CONFIGURED);
    assert_eq!(status_text("   ", "ollama", "ask"), AI_NOT_CONFIGURED);
}

/// The state and hint copy names no config path. Init's default moved from
/// the project layer to the user one and old guidance kept naming `.saya/`,
/// which nothing asserted — so it stayed wrong through a release.
#[test]
fn first_run_guidance_names_no_config_path() {
    for line in [
        AI_READY,
        AI_NOT_SET_UP,
        AI_REMEDY,
        PROJECT_READY,
        DATABASE_NONE,
        START_SETUP,
        START_AI,
        START_DATABASE,
        START_SQL,
        START_CODE_BOUND,
        START_CODE_UNBOUND,
    ] {
        assert!(
            !line.contains(".saya"),
            "guidance must not name a config path: {line}"
        );
    }
}

/// Every command the first screen names has to exist, or it is a dead end
/// in the one place a new user has nothing else to go on.
#[test]
fn first_run_guidance_names_real_commands() {
    let all = [
        AI_REMEDY,
        DATABASE_NONE,
        START_SETUP,
        START_AI,
        START_DATABASE,
        START_SQL,
        START_CODE_BOUND,
        START_CODE_UNBOUND,
    ]
    .join(" ");
    for command in [
        "/setup",
        "/connect",
        "saya demo",
        "/sql",
        "/model",
        "/provider",
        "Shift+Tab",
        "saya start",
    ] {
        assert!(all.contains(command), "guidance should offer `{command}`");
    }
    assert!(
        !all.contains("saya setup"),
        "the shell setup command is not the first screen's pointer: {all}"
    );
}

/// The three state labels pad to one width, or the three values do not
/// align down the screen.
#[test]
fn the_state_labels_are_padded_to_one_width() {
    let labels = [AI_LABEL, PROJECT_LABEL, DATABASE_LABEL];
    let width = labels[0].chars().count();
    for label in labels {
        assert_eq!(label.chars().count(), width, "ragged label: {label:?}");
    }
}

/// The unbound-workspace line states the fact and the same two remedies
/// the startup notice carries, without claiming the session is broken:
/// SQL, schema, and the database tools are unaffected.
#[test]
fn unbound_workspace_line_names_the_fact_and_the_remedy() {
    let all = NO_WORKSPACE_LINES.join(" ");
    assert!(
        all.contains("No workspace is bound"),
        "the line states the fact: {all}"
    );
    assert!(
        all.contains("file tools are unavailable"),
        "the line names why the file tools are absent: {all}"
    );
    assert!(
        all.contains("--workspace <dir>"),
        "the line names the explicit-bind remedy: {all}"
    );
    assert!(
        all.contains("git worktree"),
        "the line names the worktree remedy: {all}"
    );
}

/// The copy has to fit the columns it is painted into: the hints inside a
/// stacked 60-column box (56 inner cells) and the status lines inside an
/// 80-column one (46 inner cells).
#[test]
fn first_run_guidance_fits_its_columns() {
    for line in [
        START_SETUP,
        START_AI,
        START_DATABASE,
        START_SQL,
        START_CODE_BOUND,
        START_CODE_UNBOUND,
    ] {
        assert!(line.chars().count() <= 56, "too wide to stack: {line}");
    }
    for line in [
        AI_READY,
        AI_NOT_SET_UP,
        AI_REMEDY,
        PROJECT_READY,
        DATABASE_NONE,
    ] {
        assert!(
            line.chars().count() <= 46,
            "too wide for the right column: {line}"
        );
    }
    for line in NO_WORKSPACE_LINES {
        assert!(line.chars().count() <= 64, "too wide to centre: {line}");
    }
}
