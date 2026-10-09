//! The journal's event vocabulary: why a grant was made, why bypass
//! activated, and the tagged event lines the journal carries.

/// Why a token was granted — the `source` field of a `session-granted` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    /// A token seeded by `/allow`, journaled when it is seeded — before
    /// anything runs under it.
    Seed,
    /// A token granted by answering `[s]` at an ask, journaled when the
    /// grant lands — before the call it first allowed runs.
    Prompt,
}

/// What activated bypass — the `source` field of a `session-bypass` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BypassSource {
    /// The launch stated the mode: a fresh session under its launch mode, or
    /// a resume whose `--approval-mode` explicitly overrode the record.
    Launch,
    /// `/approvals bypass` flipped the mode mid-session.
    Command,
}

/// One journal line, tagged on `event`, so each line is exactly the settled
/// shape: `{"event":"session-granted","token":…,"source":…}` and
/// `{"event":"session-bypass","source":…}`. Serialization writes the settled
/// key order (see `append`); deserialization accepts the tag anywhere.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "event")]
#[non_exhaustive]
pub enum JournalEvent {
    #[serde(rename = "session-granted")]
    Granted { token: String, source: GrantSource },
    #[serde(rename = "session-bypass")]
    Bypass { source: BypassSource },
    /// The session's deny list, once at session start when non-empty: the
    /// denied programs, sorted, in the order the list carries them.
    #[serde(rename = "session-deny-list")]
    DenyList { programs: Vec<String> },
    /// One host call: the program named in the ask and its argv, on the door
    /// the ask entered through. Redacted through the existing seam, written
    /// before the child spawns — consent-before-action for the one lane
    /// where it matters most.
    #[serde(rename = "session-command")]
    Command {
        program: String,
        argv: Vec<String>,
        door: String,
    },
    /// One deny firing: the program named in the ask, its argv, and the door
    /// the ask entered through. Redacted through the existing seam, written
    /// before the refusal is relayed — journal-before-spawn's mirror.
    #[serde(rename = "session-command-denied")]
    CommandDenied {
        program: String,
        argv: Vec<String>,
        door: String,
    },
}
