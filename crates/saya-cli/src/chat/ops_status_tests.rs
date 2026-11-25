//! The panel and its aliases: five headings in order, one fact per line, and
//! the notes the deprecated spellings state exactly once.

use super::{Note, Section, StatusFacts, StatusNotes, StatusPanel, with_note};
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::state::SessionState;
use crate::slash::{CompactRequest, SessionsRequest, SlashCommand, parse_slash_command};

/// The facts one test session reports: a full panel, every section non-empty.
fn facts() -> StatusFacts {
    StatusFacts {
        session_id: "s-1".into(),
        record: Some("0.3 MiB of the 4 MiB limit".into()),
        run: Some("run-9 (lineage only)".into()),
        provider: "anthropic".into(),
        model: "sonnet".into(),
        context_use: Some("12% of a 200000-token window".into()),
        tokens: "Session token usage (2 turns):\n\n Input tokens: 120\n Output tokens: 40".into(),
        cost: Some("$0.0125".into()),
        workspace_root: Some("/w/saya".into()),
        host_lane: true,
        instructions: Some("Loaded:\n  project: AGENTS.md (bounded, digest 1a2b)".into()),
        limits: vec!["limits.job.no_progress_turns: 3".into()],
        database_budget: Vec::new(),
    }
}

fn parse(text: &str) -> SlashCommand {
    parse_slash_command(text)
        .unwrap_or_else(|error| panic!("`{text}` should parse: {error}"))
        .unwrap_or_else(|| panic!("`{text}` should be a slash command"))
}

fn session() -> SessionState {
    SessionState::new("s-1", None, "sonnet")
}

/// One command's answer, as the surface sees it.
fn answer(state: &mut SessionState, text: &str) -> String {
    match state.apply(parse(text), &[]) {
        SessionAction::Message(text) => text,
        other => panic!("`{text}` answers with a message, not {other:?}"),
    }
}

#[test]
fn the_panel_has_exactly_the_five_headings_in_order() {
    let panel = StatusPanel::build(&facts());
    let headings = panel
        .sections()
        .iter()
        .map(|view| view.heading())
        .collect::<Vec<_>>();
    assert_eq!(
        headings,
        vec!["Session", "Model", "Workspace", "Instructions", "Limits"],
        "the panel is five sections, in this order"
    );
    let rendered = panel.render();
    let printed = rendered
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(' '))
        .collect::<Vec<_>>();
    assert_eq!(
        printed,
        vec!["Session", "Model", "Workspace", "Instructions", "Limits"],
        "every heading prints, once, in order: {rendered}"
    );
}

#[test]
fn every_section_states_one_fact_per_line() {
    let panel = StatusPanel::build(&facts());
    let rendered = panel.render();
    for line in rendered.lines() {
        assert!(
            line.is_empty()
                || line.starts_with("  ")
                || Section::ALL.iter().any(|section| section.heading() == line),
            "a fact line is indented under its heading: {line:?}"
        );
    }
    let model = panel.render_section(Section::Model);
    assert!(model.starts_with("Model\n"), "{model}");
    assert!(model.contains("\n  Provider: anthropic\n"), "{model}");
    assert!(model.contains("\n  Model: sonnet\n"), "{model}");
    assert!(
        model.contains("\n  Context use: 12% of a 200000-token window\n"),
        "{model}"
    );
    assert!(
        model.contains("\n  Tokens: Session token usage (2 turns):"),
        "{model}"
    );
}

#[test]
fn an_absent_fact_names_why_rather_than_showing_zero() {
    let panel = StatusPanel::build(&StatusFacts {
        session_id: "s-2".into(),
        ..Default::default()
    });
    let rendered = panel.render();
    assert!(rendered.contains("\n  Record size: unknown"), "{rendered}");
    assert!(rendered.contains("\n  Run: no run link"), "{rendered}");
    assert!(rendered.contains("\n  Context use: unknown"), "{rendered}");
    assert!(rendered.contains("\n  Root: none"), "{rendered}");
    assert!(
        rendered.contains("\n  Host lane: not composed"),
        "{rendered}"
    );
    assert!(rendered.contains("\n  Sources: unknown"), "{rendered}");
    assert!(
        rendered.contains("\n  no [limits.job] guards declared"),
        "{rendered}"
    );
}

#[test]
fn the_limits_section_lists_the_job_guards_and_the_database_budget_only_when_bound() {
    let unbound = StatusPanel::build(&facts()).render_section(Section::Limits);
    assert!(
        unbound.contains("limits.job.no_progress_turns: 3"),
        "{unbound}"
    );
    assert!(
        !unbound.contains("limits.database"),
        "no database is bound, so no database budget is a fact: {unbound}"
    );
    let bound = StatusPanel::build(&StatusFacts {
        database_budget: vec!["limits.database.requests: 12".into()],
        ..facts()
    })
    .render_section(Section::Limits);
    assert!(bound.contains("limits.job.no_progress_turns: 3"), "{bound}");
    assert!(bound.contains("limits.database.requests: 12"), "{bound}");
}

#[test]
fn usage_is_its_own_panel_and_carries_the_ceiling_it_states() {
    let mut state = session();
    assert_eq!(
        state.apply(parse("/usage"), &[]),
        SessionAction::Usage(crate::slash::UsageRequest::show()),
        "bare `/usage` reads the session's own panel"
    );
    assert_eq!(
        state.apply(parse("/usage --limit 2.5"), &[]),
        SessionAction::Usage(crate::slash::UsageRequest::limit(2_500_000)),
        "`--limit` is carried to the loop, which owns the ceiling"
    );
}

#[test]
fn workspace_opens_the_workspace_section_and_says_its_note_once() {
    let mut state = session();
    let workspace =
        StatusPanel::build(&StatusFacts::of_session(&state)).render_section(Section::Workspace);
    let first = answer(&mut state, "/workspace");
    assert_eq!(first, format!("{workspace}\n{}", Note::Workspace.text()));
    assert_eq!(answer(&mut state, "/workspace"), workspace);
}

#[test]
fn context_opens_the_instructions_section_through_the_shared_renderer() {
    // `/context` needs the runtime's settings, so the surface hands the panel
    // its body; the section is that body under the one heading, and the note
    // is said once, the same way.
    let mut state = session();
    let body = "Instruction sources for the next request";
    let panel = StatusPanel::build(&StatusFacts {
        instructions: Some(body.to_owned()),
        ..StatusFacts::of_session(&state)
    });
    let section = panel.render_section(Section::Instructions);
    assert_eq!(section, format!("Instructions\n  Sources: {body}"));
    let first = with_note(&mut state, Note::Context, section.clone());
    assert_eq!(first, format!("{section}\n{}", Note::Context.text()));
    assert_eq!(
        with_note(&mut state, Note::Context, section.clone()),
        section
    );
}

#[test]
fn history_is_the_sessions_listing_under_its_own_spelling() {
    let history = parse("/history");
    let sessions = parse("/sessions");
    assert_eq!(
        history,
        SlashCommand::Sessions(SessionsRequest::history()),
        "`/history` is `/sessions` stated through the alias"
    );
    assert_eq!(sessions, SlashCommand::Sessions(SessionsRequest::default()));
    let mut state = session();
    assert_eq!(
        state.apply(history, &[]),
        SessionAction::History(true),
        "the alias asks for the same listing, marked as the alias"
    );
    assert_eq!(
        state.apply(sessions, &[]),
        SessionAction::History(false),
        "the canonical spelling asks for the same listing"
    );
    assert_eq!(
        with_note(&mut state, Note::History, "Recent sessions".to_owned()),
        format!("Recent sessions\n{}", Note::History.text())
    );
}

#[test]
fn rotate_is_compact_new_session_under_its_own_spelling() {
    let rotate = parse("/rotate");
    let compact = parse("/compact --new-session");
    assert_eq!(
        rotate,
        SlashCommand::Compact(CompactRequest::rotate()),
        "the alias states the spelling it came from, for its note"
    );
    assert_eq!(
        compact,
        SlashCommand::Compact(CompactRequest::new_session()),
        "the canonical spelling states no alias"
    );
    let (SlashCommand::Compact(alias), SlashCommand::Compact(canonical)) =
        (rotate.clone(), compact.clone())
    else {
        panic!("both spellings are the compaction command");
    };
    assert_eq!(
        (alias.new_session, canonical.new_session),
        (true, true),
        "/rotate is /compact --new-session: the same flag, hence the same operation"
    );
    assert!(!canonical.via_rotate_alias && alias.via_rotate_alias);
    let mut state = session();
    assert_eq!(
        state.apply(rotate, &[]),
        SessionAction::Rotate(CompactRequest::rotate()),
        "the alias runs the one compaction-into-a-successor operation"
    );
    assert_eq!(
        state.apply(compact, &[]),
        SessionAction::Rotate(CompactRequest::new_session()),
        "the canonical spelling runs the same operation"
    );
    assert_eq!(
        state.apply(parse("/compact"), &[]),
        SessionAction::Compact,
        "bare /compact stays the compaction of this session"
    );
    assert_eq!(
        with_note(
            &mut state,
            Note::Rotate,
            "Rotated into a new session".to_owned()
        ),
        format!("Rotated into a new session\n{}", Note::Rotate.text())
    );
}

#[test]
fn a_plain_compact_refuses_the_one_release_flags_it_does_not_know() {
    for text in [
        "/compact --session",
        "/compact new-session",
        "/compact --new-session now",
    ] {
        assert!(
            parse_slash_command(text).is_err(),
            "`{text}` names a form /compact does not have"
        );
    }
}

#[test]
fn every_note_is_said_once_per_session() {
    let mut notes = StatusNotes::default();
    for note in Note::ALL {
        assert!(notes.first_time(note), "{note:?} is owed first");
        assert!(!notes.first_time(note), "{note:?} is said once");
    }
    let mut state = session();
    for note in Note::ALL {
        assert_eq!(
            with_note(&mut state, note, "answer".to_owned()),
            format!("answer\n{}", note.text())
        );
        assert_eq!(with_note(&mut state, note, "answer".to_owned()), "answer");
    }
}
