//! The retired `saya coding` spelling, and where each of its verbs moved.
//!
//! The retired name still parses — a hidden `Coding` command captures the words
//! after it — so the answer can name the spelling that does work instead of
//! clap's "unrecognized subcommand". Nothing here runs a job; every path ends
//! in a usage error (exit 2).

use clap::Subcommand;

/// The retired top-level spelling, and the message that names its replacements.
pub(crate) const RETIRED_COMMANDS: &[(&str, &str)] = &[(
    "coding",
    "`saya coding` was replaced: `saya start` starts a job, `saya job` runs its \
     lifecycle verbs, and the session-only controls are `/approve`, `/deny`, \
     `/message`, `/pause`, `/continue`, and `/cancel-child`",
)];

/// All 28 old `saya coding` verbs, and the spelling that runs each one. The
/// eight agent-shaped controls have no top-level spelling: they are the
/// interactive session's own commands.
const RETIRED_CODING_SUBCOMMANDS: &[(&str, &str)] = &[
    (
        "export",
        "`saya coding export` moved: use `saya job export`",
    ),
    (
        "events",
        "`saya coding events` moved: use `saya job events`",
    ),
    (
        "review",
        "`saya coding review` moved: use `saya job review`",
    ),
    (
        "review-existing",
        "`saya coding review-existing` moved: use `saya job review-existing`",
    ),
    ("apply", "`saya coding apply` moved: use `saya job apply`"),
    (
        "remove-checkout",
        "`saya coding remove-checkout` moved: use `saya job remove-checkout`",
    ),
    (
        "reconcile-checkout",
        "`saya coding reconcile-checkout` moved: use `saya job reconcile-checkout`",
    ),
    (
        "retain-checkout",
        "`saya coding retain-checkout` moved: use `saya job retain-checkout`",
    ),
    (
        "resume",
        "`saya coding resume` moved: use `saya job resume`",
    ),
    ("notes", "`saya coding notes` moved: use `saya job notes`"),
    (
        "accept",
        "`saya coding accept` moved: use `saya job accept`",
    ),
    (
        "promote",
        "`saya coding promote` moved: use `saya job promote`",
    ),
    (
        "approve",
        "`saya coding approve` is session-only: use `/approve <id>` in the \
         interactive session",
    ),
    (
        "deny",
        "`saya coding deny` is session-only: use `/deny <id>` in the interactive \
         session",
    ),
    (
        "follow-up",
        "`saya coding follow-up` moved: use `saya follow-up <id> <agent> …`",
    ),
    (
        "message",
        "`saya coding message` is session-only: use `/message <id> <agent> \
         <text>` in the interactive session",
    ),
    (
        "pause",
        "`saya coding pause` is session-only: use `/pause <id> <agent>` in the \
         interactive session",
    ),
    (
        "continue",
        "`saya coding continue` is session-only: use `/continue <id> <agent>` in \
         the interactive session",
    ),
    (
        "cancel-child",
        "`saya coding cancel-child` is session-only: use `/cancel-child <id> \
         <agent>` in the interactive session",
    ),
    (
        "start",
        "`saya coding start` moved: use `saya start <id> <task> …` (the same \
         request and flags)",
    ),
    ("list", "`saya coding list` moved: use `saya job list`"),
    ("show", "`saya coding show` moved: use `saya job show`"),
    (
        "cancel",
        "`saya coding cancel` moved: use `saya job cancel`",
    ),
    (
        "doctor",
        "`saya coding doctor` moved: use `saya job doctor`",
    ),
    ("setup", "`saya coding setup` moved: use `saya job setup`"),
    (
        "reconcile",
        "`saya coding reconcile` moved: use `saya job reconcile`",
    ),
    (
        "worktrees",
        "`saya coding worktrees` moved: use `saya job worktrees`",
    ),
    (
        "rewind",
        "`saya coding rewind` moved: use `saya job rewind`",
    ),
];

/// The words clap captured after the retired `coding` name.
///
/// The variant is the derive's required newtype shape: an external
/// subcommand is declared on a variant, and it captures every following word
/// verbatim (including flags) so the retired grammar never has to be restated.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
#[command(about = "The words after the retired `coding` name.")]
pub enum RetiredCoding {
    #[command(external_subcommand)]
    Words(Vec<String>),
}

impl RetiredCoding {
    /// The captured words, starting with the retired verb.
    pub(crate) fn words(&self) -> &[String] {
        match self {
            Self::Words(words) => words,
        }
    }
}

/// The one answer a retired spelling gets: exit 2, naming where the verb
/// moved. An empty tail, or one that starts with a flag (the retired
/// `saya coding --repository … <verb>` shape), falls back to the top-level
/// message rather than reading a flag as a verb.
pub(crate) fn moved_message(command: Option<&RetiredCoding>) -> String {
    let Some(verb) = command
        .and_then(|command| command.words().first())
        .filter(|verb| !verb.starts_with('-'))
    else {
        return RETIRED_COMMANDS[0].1.to_owned();
    };
    RETIRED_CODING_SUBCOMMANDS
        .iter()
        .find(|(name, _)| name == verb)
        .map_or_else(
            || {
                format!(
                    "`saya coding {verb}` is not a verb: `saya start` starts a job \
                     and `saya job` runs its lifecycle verbs"
                )
            },
            |(_, message)| (*message).to_owned(),
        )
}
