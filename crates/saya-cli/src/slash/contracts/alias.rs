//! The knowledge store's one-release spellings: the note each deprecated
//! spelling states, and the request type that carries it.
//!
//! An alias is not a second command. `/queue`, `/confirm`, `/reject`,
//! `/approve-all` and `/contract` parse into **the same** `ContractsCommand`
//! `/contracts <verb>` parses, and the one dispatcher, `run_contracts`, runs
//! them — same store call, same rendering. The only difference is the note the
//! deprecated spelling appends to that output, once per command.
//!
//! Every note is one line and names the spelling that replaces it. The notes
//! live here, beside the request type, so one adapter cannot print a note
//! another forgot.

use crate::cli::ContractsCommand;

/// The one-line note the deprecated `/contract` spelling appends: `/contracts`
/// spells the same operation.
pub(crate) const CONTRACT_ALIAS_NOTE: &str = "note: `/contract` is a deprecated alias of \
     `/contracts` and is removed in the next release: use `/contracts list` or \
     `/contracts show <table>`.";

/// The one-line note the deprecated `/queue` spelling appends.
pub(crate) const QUEUE_ALIAS_NOTE: &str = "note: `/queue` is a deprecated alias of \
     `/contracts queue` and is removed in the next release: use `/contracts queue [limit]`.";

/// The one-line note the deprecated `/confirm` spelling appends.
pub(crate) const CONFIRM_ALIAS_NOTE: &str = "note: `/confirm` is a deprecated alias of \
     `/contracts confirm` and is removed in the next release: use \
     `/contracts confirm <claim-id-prefix>`.";

/// The one-line note the deprecated `/reject` spelling appends.
pub(crate) const REJECT_ALIAS_NOTE: &str = "note: `/reject` is a deprecated alias of \
     `/contracts reject` and is removed in the next release: use \
     `/contracts reject <claim-id-prefix>`.";

/// The one-line note the deprecated `/approve-all` spelling appends. It says
/// what the command is about: `/approve-all` reads like job approvals, and it
/// never was one.
pub(crate) const APPROVE_ALL_ALIAS_NOTE: &str = "note: `/approve-all` is a deprecated alias of \
     `/contracts confirm-all` and is removed in the next release: use \
     `/contracts confirm-all [--yes] [limit]`. It confirms knowledge claims, never job \
     approvals — those are `/approve <id>`.";

/// The one operation a contract slash command names, plus the one-line note
/// its deprecated spelling states.
///
/// Every spelling parses into this, so the store call and the rendering are
/// stated once; only the note distinguishes a deprecated spelling from the
/// canonical one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractsRequest {
    /// The operation: exactly the `ContractsCommand` the headless
    /// `saya contracts` parser produces for the same operation.
    pub command: ContractsCommand,
    /// The one-line deprecation note this spelling appends to the output,
    /// once; `None` for a canonical spelling and for `/remember`/`/forget`.
    pub note: Option<&'static str>,
}

impl ContractsRequest {
    /// A request stated through `spelling`; a deprecated spelling carries its
    /// note.
    pub(crate) fn stating(spelling: &str, command: ContractsCommand) -> Self {
        Self {
            command,
            note: note_for(spelling),
        }
    }
}

/// The deprecation note `spelling` states, or `None` for a canonical spelling.
///
/// The five deprecated spellings are the knowledge store's one-release
/// aliases; `/remember` and `/forget` are canonical short verbs and are not
/// deprecated, so they state nothing.
pub(crate) fn note_for(spelling: &str) -> Option<&'static str> {
    match spelling {
        "contract" => Some(CONTRACT_ALIAS_NOTE),
        "queue" => Some(QUEUE_ALIAS_NOTE),
        "confirm" => Some(CONFIRM_ALIAS_NOTE),
        "reject" => Some(REJECT_ALIAS_NOTE),
        "approve-all" => Some(APPROVE_ALL_ALIAS_NOTE),
        _ => None,
    }
}
