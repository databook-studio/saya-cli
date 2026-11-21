//! Where a job was opened from.
//!
//! A chat session and every `saya ask` open a job; so does every terminal verb.
//! The origin says which, so `saya job list` can show the CLI's own jobs by
//! default. It is a listing label and never authority: a session job's own
//! `show`, `continue` and `rewind` work on it exactly as a CLI job's do.

use serde::{Deserialize, Serialize};

/// The surface that opened a job. `Cli` is the default, so a `request.json`
/// written before the key existed — and one a terminal verb writes today —
/// states no origin at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOrigin {
    #[default]
    Cli,
    Session,
}

impl JobOrigin {
    pub(crate) fn is_cli(&self) -> bool {
        *self == Self::Cli
    }
    /// The word a listing shows.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Session => "session",
        }
    }
}
