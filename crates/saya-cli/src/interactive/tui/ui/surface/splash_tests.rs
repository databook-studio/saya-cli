use super::*;

/// Every row must be the same display width, or centring skews the figure.
#[test]
fn splash_rows_are_padded_to_one_width() {
    for art in [&SPLASH_ART[..], &SPLASH_ART_COMPACT[..]] {
        let width = art[0].chars().count();
        for row in art {
            assert_eq!(row.chars().count(), width, "ragged row: {row:?}");
        }
    }
}

/// The art shrinks, then disappears, rather than pushing the splash text
/// off the top.
#[test]
fn splash_art_yields_to_the_text_when_the_terminal_is_short() {
    // Room to spare: the full mascot.
    assert_eq!(splash_art(40, 11).map(|a| a.len()), Some(SPLASH_ART.len()));
    // One line short of the full mascot: fall back to the compact one.
    assert_eq!(
        splash_art(11 + SPLASH_ART.len(), 11).map(|a| a.len()),
        Some(SPLASH_ART_COMPACT.len())
    );
    // One line short of the compact mascot: the text draws alone.
    assert!(splash_art(11 + SPLASH_ART_COMPACT.len(), 11).is_none());
    assert!(splash_art(0, 11).is_none());
}

/// The guidance must not name a config path. Init's default moved from the
/// project layer to the user one and this copy kept naming `.saya/`, which
/// nothing asserted — so it stayed wrong through a release.
#[test]
fn first_run_guidance_names_no_config_path() {
    for line in NO_DATABASE_STEPS
        .iter()
        .chain([&NO_DATABASE_HEADLINE, &NO_DATABASE_FOOTER])
    {
        assert!(
            !line.contains(".saya"),
            "guidance must let `config init` report the path: {line}"
        );
    }
}

/// Every command the guidance names has to exist, or it is a dead end in the
/// one place a new user has nothing else to go on.
#[test]
fn first_run_guidance_names_real_commands() {
    let all = NO_DATABASE_STEPS.join(" ") + NO_DATABASE_FOOTER;
    for command in [
        "saya config init",
        "saya connection test",
        "saya config doctor",
    ] {
        assert!(all.contains(command), "guidance should offer `{command}`");
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

/// The splash is centred, so a line wider than a narrow terminal wraps and
/// breaks the centring for every line under it.
#[test]
fn first_run_guidance_fits_a_narrow_terminal() {
    for line in NO_DATABASE_STEPS
        .iter()
        .chain([&NO_DATABASE_HEADLINE, &NO_DATABASE_FOOTER])
        .chain(&NO_WORKSPACE_LINES)
    {
        assert!(line.chars().count() <= 64, "too wide to centre: {line}");
    }
}

/// Both pupils must survive styling. The previous builder split the row on
/// the first cursor glyph and emitted three spans, which silently dropped
/// the second eye the moment the mascot grew one.
#[test]
fn both_pupils_are_styled_as_cursors() {
    for art in [&SPLASH_ART[..], &SPLASH_ART_COMPACT[..]] {
        let lines = art_lines(art);
        let bold: Vec<String> = lines
            .iter()
            .flat_map(|line| &line.spans)
            .filter(|span| span.style.add_modifier.contains(Modifier::BOLD))
            .map(|span| span.content.to_string())
            .collect();
        assert_eq!(
            bold,
            vec!["\u{2590}", "\u{258c}"],
            "expected exactly two pupils"
        );
    }
}

/// A row must round-trip: coalescing runs may change how the text is split
/// into spans, never which characters reach the screen.
#[test]
fn styling_preserves_every_glyph_in_the_row() {
    for art in [&SPLASH_ART[..], &SPLASH_ART_COMPACT[..]] {
        for (row, line) in art.iter().zip(art_lines(art)) {
            let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(&rendered, row, "row must survive styling unchanged");
        }
    }
}

/// Beak and talons recede; the body carries the accent. Without this a glyph
/// added to the art silently inherits the body colour.
#[test]
fn beak_and_talons_recede_behind_the_body() {
    let secondary_style = Style::default().fg(secondary());
    for ch in ['\u{25bc}', '\u{2580}'] {
        assert_eq!(glyph_style(ch), secondary_style, "{ch} should recede");
    }
    assert_eq!(glyph_style('\u{2588}'), Style::default().fg(accent()));
}
