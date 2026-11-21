//! The transcript's empty states: the four screens `02` §9 draws before the
//! conversation starts.
//!
//! A pure function of the facts the session holds and the pane's width and
//! height. Nothing here reads the terminal, the clock or the model: the caller
//! says whether a provider answers, which database is connected and which
//! project is bound, and the pane answers with the rows to paint. The examples
//! of a connected database are today's `starter_questions` for it, read rather
//! than copied, so a question the schema changes is a question the screen
//! changes.

use ratatui::text::Line;

use super::empty_lines::{connected_rows, no_database_rows, no_workspace_rows};
use super::empty_welcome::welcome_rows;
use crate::front::tui::theme::{Charset, Scheme};

/// The database the run is connected to, as `02` §9.3's header names it: the
/// name, the dialect and how many tables the cached schema holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Database {
    pub(crate) name: String,
    pub(crate) dialect: String,
    pub(crate) tables: usize,
}

/// The facts an empty transcript is painted from.
#[derive(Debug, Clone)]
pub(crate) struct EmptyFacts {
    /// Whether a provider is configured. When it is not, the first run says so
    /// and offers the three ways in.
    pub(crate) provider: bool,
    /// The database the run is connected to, if one is.
    pub(crate) database: Option<Database>,
    /// The workspace root as the caller abbreviates it (`~/shop`); `None` when
    /// no project is bound, which is the state that turns file tools off.
    pub(crate) workspace: Option<String>,
    /// The questions the connected database suggests, as today's
    /// `starter_questions` picked them from the cached schema.
    pub(crate) starter_questions: Vec<String>,
    /// Which token column paints the rows.
    pub(crate) scheme: Scheme,
    /// Which glyph set the rows draw with.
    pub(crate) charset: Charset,
}

impl Default for EmptyFacts {
    fn default() -> Self {
        Self {
            provider: false,
            database: None,
            workspace: None,
            starter_questions: Vec::new(),
            scheme: Scheme::Dark,
            charset: Charset::Utf8,
        }
    }
}

/// The four empty states of `02` §9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// `02` §9.1: no provider configured, so nothing can answer yet.
    NoProvider,
    /// `02` §9.2: ready, with no database connected.
    NoDatabase,
    /// `02` §9.3: ready, with a database connected.
    Connected,
    /// `02` §9.4: ready, with no workspace bound — file tools are off.
    NoWorkspace,
}

/// The state the facts select.
///
/// A screen that cannot answer outranks every other account of the session,
/// and a session that can edit no file says so before it describes its
/// database: `02` §9.1, then §9.4, then §9.3, then §9.2.
pub(crate) fn state(facts: &EmptyFacts) -> State {
    if !facts.provider {
        State::NoProvider
    } else if facts.workspace.is_none() {
        State::NoWorkspace
    } else if facts.database.is_some() {
        State::Connected
    } else {
        State::NoDatabase
    }
}

/// The transcript's rows for the state the facts select, fitted to the pane.
///
/// The pane never paints more rows than it was given: a state shorter than the
/// transcript area leaves the room below it to the composer's neighbours.
pub(crate) fn rows(facts: &EmptyFacts, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut rows = match state(facts) {
        State::NoProvider => welcome_rows(facts, width),
        State::NoDatabase => no_database_rows(facts, width),
        State::Connected => connected_rows(facts, width),
        State::NoWorkspace => no_workspace_rows(facts, width),
    };
    rows.truncate(height);
    rows
}
