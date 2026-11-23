//! The accessibility pass (`06` §5, T20): meaning never lives in colour alone,
//! `NO_COLOR` is honoured once at launch, a narrow frame keeps every caveat and
//! every answering key, and `--ascii` leaves no glyph a terminal cannot draw.
//!
//! The panes are pure functions, so the whole pass is assertions over their
//! words and over the one frame `run_draw` paints.

use std::collections::BTreeMap;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use saya_agent::PermissionMode;

use super::a11y_fixture::{self, Pane};
use super::jobs::JobReport;
use super::model::Model;
use super::run_draw::{Facts, frame};
use super::theme::{Charset, NO_COLOR, Scheme, scheme};
use super::view::cards_fixture::shell_question;
use super::view::jobs_fixture;
use crate::chat::Question;

/// The screen `model` paints at `width × height`, one row per line.
fn painted(model: &Model, facts: &Facts, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, facts, area))
        .expect("the frame is painted");
    format!("{}", terminal.backend())
        .lines()
        .map(|row| row.trim_matches('"').to_owned())
        .collect()
}

/// A conversation on screen: the operator's question and a draft.
fn conversation() -> Model {
    let mut model = Model::default();
    model.begin_turn(String::from("how many orders did we get?"));
    model.insert("a follow-up");
    model
}

/// A call waiting to be answered, on top of the conversation.
fn questioning() -> Model {
    let mut model = conversation();
    model.show_card(shell_question());
    model
}

/// The launch posture a narrow frame still has to state: both caveats, a deny
/// list and a model id.
fn posture() -> Facts {
    Facts {
        workspace: Some(String::from("~/shop")),
        model: String::from("deepseek-v4.1-flash"),
        host_unsandboxed: true,
        sharing: true,
        deny: vec![String::from("curl"), String::from("rm")],
        ..Facts::default()
    }
}

/// Every pane of `02` §2-§11 renders with colour off into the same words as its
/// coloured render: `02` §12 rule 2, over the set rather than one screen.
#[test]
fn every_pane_says_the_same_words_with_colour_off() {
    for pane in a11y_fixture::ALL {
        let coloured = a11y_fixture::render(pane, Scheme::Dark, Charset::Utf8);
        let plain = a11y_fixture::render(pane, Scheme::NoColour, Charset::Utf8);
        assert!(
            !coloured.is_empty(),
            "the {} paints rows, so comparing them means something",
            pane.name()
        );
        assert_eq!(
            coloured,
            plain,
            "{}: colour off is the same words (02 §12)",
            pane.name()
        );
    }
}

/// The bypass chip is reverse video *and* a word: with colour gone, the word
/// is the whole signal (`01` §4, `02` §4).
#[test]
fn the_bypass_chip_keeps_its_word_with_colour_off() {
    for scheme in [Scheme::Dark, Scheme::Light, Scheme::NoColour] {
        let words = a11y_fixture::words(Pane::Status, scheme, Charset::Utf8);
        assert!(
            words.contains("! BYPASS"),
            "{scheme:?}: the chip keeps its word rather than relying on its hue: {words}"
        );
    }
}

/// no-color.org: *any* non-empty value of `NO_COLOR` asks for no colour, and an
/// unset or empty variable states nothing. The resolver reads an explicit map,
/// so the test never mutates the process environment.
#[test]
fn no_color_selects_the_no_colour_column() {
    for value in ["1", "0", "true", "yes", " "] {
        let env = BTreeMap::from([(String::from(NO_COLOR), String::from(value))]);
        assert_eq!(
            scheme(&env, Scheme::Dark),
            Scheme::NoColour,
            "NO_COLOR={value:?} asks for no colour"
        );
        assert_eq!(
            scheme(&env, Scheme::Light),
            Scheme::NoColour,
            "NO_COLOR={value:?} outranks the light palette"
        );
    }
}

/// An absent or empty `NO_COLOR` leaves the chosen palette alone.
#[test]
fn an_unset_or_empty_no_color_keeps_the_chosen_palette() {
    let empty = BTreeMap::new();
    assert_eq!(scheme(&empty, Scheme::Dark), Scheme::Dark);
    assert_eq!(scheme(&empty, Scheme::Light), Scheme::Light);

    let blank = BTreeMap::from([(String::from(NO_COLOR), String::new())]);
    assert_eq!(
        scheme(&blank, Scheme::Dark),
        Scheme::Dark,
        "an empty value is not a request"
    );
}

/// 80x24 and 60x20: the status line never truncates a caveat, and the docked
/// card's keys row is fully visible (`02` §4, §5).
#[test]
fn narrow_frames_keep_every_caveat_and_every_answering_key() {
    for (width, height) in [(80, 24), (60, 20)] {
        let mut model = questioning();
        model.set_mode(PermissionMode::Bypass);
        let rows = painted(&model, &posture(), width, height);

        let status = rows
            .iter()
            .find(|row| row.contains("BYPASS"))
            .unwrap_or_else(|| panic!("{width}x{height}: no status line in {rows:#?}"));
        assert!(
            status.contains("host unsandboxed"),
            "{width}x{height}: the host caveat is on the status line: {status:?}"
        );
        assert!(
            status.contains("sharing"),
            "{width}x{height}: the data-sharing caveat is on the status line: {status:?}"
        );

        let screen = rows.join("\n");
        for key in [
            "[y] Allow once",
            "[s] Allow for this session",
            "shell:python3",
            "[n] Deny",
            "[Esc] Deny",
        ] {
            assert!(
                screen.contains(key),
                "{width}x{height}: the panel says {key:?}: {screen}"
            );
        }
    }
}

/// `--ascii`: every byte of every frame is seven-bit, so a terminal that cannot
/// draw the UTF-8 set still reads the whole screen (`02` §11).
#[test]
fn every_ascii_frame_is_seven_bit() {
    let mut jobs = conversation();
    jobs.note_jobs(JobReport::Listed(jobs_fixture::jobs()));
    jobs.open_jobs();

    let mut picker = conversation();
    picker.open_mode_picker();

    let mut trust = Model::default();
    trust.open_trust();

    let mut toasting = conversation();
    toasting.say("a refused key");

    let cases: Vec<(&str, Model)> = vec![
        ("empty", Model::default()),
        ("conversation", conversation()),
        ("card", questioning()),
        ("jobs drawer", jobs),
        ("mode picker", picker),
        ("trust question", trust),
        ("toast", toasting),
    ];
    for (name, model) in cases {
        let facts = Facts {
            charset: Charset::Ascii,
            ..posture()
        };
        let screen = painted(&model, &facts, 80, 24).join("\n");
        if let Some(byte) = screen.bytes().find(|byte| !byte.is_ascii()) {
            panic!("the {name} frame is not seven-bit: byte {byte:#04x} in {screen}");
        }
    }
}

/// The card is the one pane whose call and answers are their own rows: the
/// sample says so, which is what makes the narrow-frame assertion above about
/// the panel rather than a clipped tail.
#[test]
fn the_card_states_the_call_and_its_answers_on_separate_rows() {
    let rows = a11y_fixture::render(Pane::Card, Scheme::NoColour, Charset::Utf8);
    let place = rows
        .iter()
        .position(|row| row.starts_with("in "))
        .expect("the card says where the call runs");
    let command = rows
        .iter()
        .position(|row| row.starts_with("$ "))
        .expect("the card states the command");
    let allow = rows
        .iter()
        .position(|row| row.starts_with("[y] Allow once"))
        .expect("the card says how it is answered");
    assert!(place < command, "the place sits above the call: {rows:#?}");
    assert!(command < allow, "and the answers under it: {rows:#?}");
    assert!(
        !rows[command].contains("[n]"),
        "the call's row carries no answer: {:?}",
        rows[command]
    );
}

/// The `Question` type is the card's own input, so the frame test above states
/// the same ask the card pane does.
#[test]
fn the_shell_question_names_a_session_grant() {
    let question: Question = shell_question();
    assert_eq!(question.grant.as_deref(), Some("shell:python3"));
}
