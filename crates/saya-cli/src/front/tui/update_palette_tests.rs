//! `02` §11's command palette through the pure update: what `Ctrl+K` and a bare
//! `/` open, what typing narrows it to, how the selection walks the list, what
//! Enter does with the selected command, and what Esc does — nothing.
//!
//! The palette's Enter is asserted against the line it stands for: the command
//! it runs must produce the commands typing that line produces, because both go
//! through the composer's one submit.

use ratatui::crossterm::event::KeyCode;

use super::model::Model;
use super::msg::Cmd;
use super::update::update;
use super::update_fixture::{ctrl_k, enter, escape, press, typed};
use crate::cli::CodingCommand;
use crate::slash::SlashCommand;

/// The name the palette's selected row carries, if the palette is open.
fn selected(model: &Model) -> Option<&'static str> {
    model
        .palette()
        .and_then(|palette| palette.selected_entry())
        .map(|entry| entry.name)
}

#[test]
fn ctrl_k_opens_the_palette_and_tells_the_session_nothing() {
    let mut model = Model::default();
    let cmds = update(&mut model, ctrl_k());

    assert!(cmds.is_empty(), "opening a pane is not a command: {cmds:?}");
    assert!(model.overlay_open(), "Ctrl+K opens a pane");
    let palette = model
        .palette()
        .expect("the pane Ctrl+K opens is the palette");
    assert_eq!(palette.query(), "", "it opens on nothing typed");
    assert_eq!(
        model.composer(),
        "",
        "the palette's query is the pane's, not a draft"
    );
    assert!(
        !model.is_busy(),
        "the key is the pane's: no turn was opened"
    );
}

#[test]
fn typing_narrows_the_palette_to_the_commands_it_matches() {
    let mut model = Model::default();
    update(&mut model, ctrl_k());

    typed(&mut model, "sta");

    // `sta` nests inside `/start`, `/status` and `/investigation` — the three
    // names carrying s, t and a in that order — and equal scores keep the
    // catalogue's order, so `/start` is the row the list opens on.
    assert_eq!(
        model
            .palette()
            .expect("the palette is open")
            .entries()
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["start", "status", "investigation"],
        "typing narrows the list to the commands the letters reach"
    );
    assert_eq!(selected(&model), Some("start"), "the best row is selected");
    assert_eq!(
        model.composer(),
        "",
        "a key typed into the palette never reaches the composer behind it"
    );

    typed(&mut model, "tu");
    assert_eq!(
        selected(&model),
        Some("status"),
        "`/status` is the one command `statu` reaches"
    );
}

#[test]
fn up_and_down_walk_the_palettes_rows_and_stop_at_both_ends() {
    let mut model = Model::default();
    update(&mut model, ctrl_k());
    let first = selected(&model).expect("an empty query offers the catalogue");

    update(&mut model, press(KeyCode::Down));
    let second = selected(&model).expect("the second row is offered");
    assert_ne!(first, second, "down moves the selection");

    update(&mut model, press(KeyCode::Up));
    assert_eq!(selected(&model), Some(first), "up comes back to the first");
    update(&mut model, press(KeyCode::Up));
    assert_eq!(selected(&model), Some(first), "and stops there");

    typed(&mut model, "zzzz");
    assert_eq!(
        selected(&model),
        None,
        "a query matching nothing selects nothing"
    );
    update(&mut model, press(KeyCode::Down));
    assert_eq!(selected(&model), None, "and walking that list cannot panic");
}

#[test]
fn enter_runs_the_selected_command_exactly_as_typing_it_would() {
    let mut model = Model::default();
    update(&mut model, ctrl_k());
    typed(&mut model, "statu");
    assert_eq!(selected(&model), Some("status"));

    let chosen = update(&mut model, enter());

    let mut hand_typed = Model::default();
    hand_typed.insert("/status");
    let typed_cmds = update(&mut hand_typed, enter());
    assert_eq!(
        chosen, typed_cmds,
        "a chosen command and a typed one take one path"
    );
    assert_eq!(chosen, vec![Cmd::Op(SlashCommand::Status)]);
    assert!(
        !model.overlay_open(),
        "running the chosen command closes the palette"
    );
    assert_eq!(model.composer(), "", "and leaves no line behind");
    assert!(!model.is_busy(), "`/status` is the session's, never a turn");
}

#[test]
fn a_command_that_asks_for_an_argument_fills_the_composer_instead() {
    let mut model = Model::default();
    update(&mut model, ctrl_k());
    typed(&mut model, "sql");
    assert_eq!(selected(&model), Some("sql"));

    let cmds = update(&mut model, enter());

    assert!(
        cmds.is_empty(),
        "nothing runs until the query is typed: {cmds:?}"
    );
    assert!(!model.overlay_open(), "the palette gives the composer back");
    assert_eq!(
        model.composer(),
        "/sql ",
        "the command waits in the composer for its argument"
    );
    assert!(!model.is_busy(), "and nothing was sent");
}

#[test]
fn esc_closes_the_palette_without_a_command() {
    let mut model = Model::default();
    update(&mut model, ctrl_k());
    typed(&mut model, "sta");

    let cmds = update(&mut model, escape());

    assert!(cmds.is_empty(), "Esc asks for nothing: {cmds:?}");
    assert!(!model.overlay_open(), "the palette is closed");
    assert_eq!(model.composer(), "", "and no command was pasted into it");
}

#[test]
fn a_bare_slash_on_an_empty_composer_opens_the_palette() {
    let mut model = Model::default();
    let cmds = update(&mut model, press(KeyCode::Char('/')));

    assert!(cmds.is_empty(), "opening a pane is not a command: {cmds:?}");
    assert!(model.palette().is_some(), "`/` is the palette's own key");
    assert_eq!(
        model.composer(),
        "",
        "the slash opens the pane instead of being typed"
    );
}

#[test]
fn a_slash_with_a_draft_is_typed_into_it() {
    let mut model = Model::default();
    model.insert("/sql select 1");

    update(&mut model, press(KeyCode::Char('/')));

    assert!(!model.overlay_open(), "a draft keeps its own characters");
    assert_eq!(model.composer(), "/sql select 1/");
}

#[test]
fn a_line_that_is_the_whole_command_runs_from_the_palette_as_it_would_typed() {
    // A job's line carries its own arguments — the start's id, task and write
    // paths — so the pane's Enter must hand the line on whole, rather than
    // filling the composer with a name the tail would then be lost from.
    let line = "/start --id pty-job 'controlled task with helpers' --write a.txt";
    let mut model = Model::default();
    update(&mut model, ctrl_k());
    typed(&mut model, line.trim_start_matches('/'));

    let chosen = update(&mut model, enter());

    let mut hand_typed = Model::default();
    hand_typed.insert(line);
    assert_eq!(
        chosen,
        update(&mut hand_typed, enter()),
        "the pane's line and the typed line take one submit"
    );
    let [Cmd::Job(request)] = &chosen[..] else {
        panic!("a start is the job's own command, never a turn: {chosen:?}");
    };
    let CodingCommand::Start {
        id,
        task,
        write_paths,
        ..
    } = &request.command
    else {
        panic!("a start is a start: {request:?}");
    };
    assert_eq!(id, "pty-job", "the id the line stated");
    assert_eq!(task, "controlled task with helpers", "and its task");
    assert_eq!(
        write_paths,
        &[String::from("a.txt")],
        "and the paths it may write, none of them dropped"
    );
    assert!(!model.overlay_open(), "the pane gives the surface back");
    assert_eq!(model.composer(), "", "and leaves no line behind");
}
