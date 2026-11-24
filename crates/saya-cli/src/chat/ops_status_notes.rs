//! The panel's vocabulary: its five headings, and the one-line notes the
//! deprecated spellings state — once each per session.

use crate::interactive::session::state::SessionState;

/// One heading of the panel, in the order the panel prints them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    /// The session's identity, its record, and the run it belongs to.
    Session,
    /// The provider, the model, the context in use, and the token totals.
    Model,
    /// The bound workspace root and whether the host lane composed.
    Workspace,
    /// The instruction sources the next request would load.
    Instructions,
    /// The ceilings in force: the run's job guards and the database budget.
    Limits,
}

impl Section {
    /// Every heading, in print order.
    pub(crate) const ALL: [Section; 5] = [
        Section::Session,
        Section::Model,
        Section::Workspace,
        Section::Instructions,
        Section::Limits,
    ];

    /// The heading this section prints.
    pub(crate) const fn heading(self) -> &'static str {
        match self {
            Section::Session => "Session",
            Section::Model => "Model",
            Section::Workspace => "Workspace",
            Section::Instructions => "Instructions",
            Section::Limits => "Limits",
        }
    }

    /// Where this section sits in [`Section::ALL`], which is its print order.
    pub(crate) const fn index(self) -> usize {
        match self {
            Section::Session => 0,
            Section::Model => 1,
            Section::Workspace => 2,
            Section::Instructions => 3,
            Section::Limits => 4,
        }
    }
}

/// The one-line note a deprecated spelling states, once per session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Note {
    /// `/usage` — the panel's Model section.
    Usage,
    /// `/workspace` — the panel's Workspace section.
    Workspace,
    /// `/context` — the panel's Instructions section.
    Context,
    /// `/history` — the `/sessions` listing.
    History,
    /// `/rotate` — `/compact --new-session`.
    Rotate,
    /// `/grants` — the `/permissions` view.
    Grants,
    /// `/report` — `/export … --format md`.
    Report,
}

impl Note {
    /// Every note, in the order the enum declares them.
    pub(crate) const ALL: [Note; 7] = [
        Note::Usage,
        Note::Workspace,
        Note::Context,
        Note::History,
        Note::Rotate,
        Note::Grants,
        Note::Report,
    ];

    /// The note's own sentence, mentioning the canonical spelling and naming
    /// this release as the last one that answers the old one.
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Note::Usage => {
                "note: /usage is now the Model section of /status — this release is the last to \
                 answer it"
            }
            Note::Workspace => {
                "note: /workspace is now the Workspace section of /status — this release is the \
                 last to answer it"
            }
            Note::Context => {
                "note: /context is now the Instructions section of /status — this release is the \
                 last to answer it"
            }
            Note::History => {
                "note: /history is now /sessions — this release is the last to answer it"
            }
            Note::Rotate => {
                "note: /rotate is now /compact --new-session — this release is the last to answer \
                 it"
            }
            Note::Grants => {
                "note: /grants is now /permissions — this release is the last to answer it"
            }
            Note::Report => {
                "note: /report is now /export --format md — this release is the last to answer it"
            }
        }
    }

    /// Where this note sits in [`Note::ALL`], which is where the session
    /// records that it has been said.
    pub(crate) const fn index(self) -> usize {
        match self {
            Note::Usage => 0,
            Note::Workspace => 1,
            Note::Context => 2,
            Note::History => 3,
            Note::Rotate => 4,
            Note::Grants => 5,
            Note::Report => 6,
        }
    }
}

/// Which notes this session has already said: each one is said the first time
/// the command that carries it is run, and never again.
///
/// In-memory only: carried on [`SessionState`] behind `#[serde(skip)]`, so a
/// resumed session starts with every note owed — a person who resumes has not
/// read this process's answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StatusNotes {
    said: [bool; Note::ALL.len()],
}

impl StatusNotes {
    /// Whether `note` is owed now, marking it said: true on the first call for
    /// that note in this session, false afterwards.
    pub(crate) fn first_time(&mut self, note: Note) -> bool {
        let said = &mut self.said[note.index()];
        let owed = !*said;
        *said = true;
        owed
    }
}

/// `text` with `note`'s sentence appended, the first time this session says it.
///
/// The sentence rides the answer rather than preceding it, so the operation's
/// own words are the first thing read — the shape `with_alias_note` gives
/// `/approvals`.
pub(crate) fn with_note(state: &mut SessionState, note: Note, text: String) -> String {
    if state.status_notes.first_time(note) {
        format!("{text}\n{}", note.text())
    } else {
        text
    }
}
