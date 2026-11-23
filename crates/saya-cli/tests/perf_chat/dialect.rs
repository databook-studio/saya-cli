//! Which spelling of the run's read-only posture the binary under test knows.
//!
//! The control arm is the pre-chat-session build: it launches in the same
//! posture with `--approval-mode read-only` and rejects `--mode` outright
//! (`error: unexpected argument '--mode' found`). The candidate states that one
//! posture — and only that one — as `--mode read-only`. The harness reads the
//! binary's own `--help` once, picks the spelling that binary documents, and
//! the record states which one each arm used, so both arms are driven with the
//! same setting over the same workload and only the flag's name differs.

use std::path::Path;
use std::process::Command;

/// The posture both arms are launched in: a perf turn asks a database
/// question, and a read-only posture keeps every approval out of the way
/// without granting a write.
const POSTURE: &str = "read-only";

/// The spelling one binary knows for [`POSTURE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Posture {
    /// `--mode read-only`: the candidate's one consent axis.
    Mode,
    /// `--approval-mode read-only`: the control's flag for the same posture.
    ApprovalMode,
}

impl Posture {
    /// The flag's name, as the record and `compare.md` state it.
    pub(crate) fn flag(self) -> &'static str {
        match self {
            Self::Mode => "--mode",
            Self::ApprovalMode => "--approval-mode",
        }
    }

    /// The two arguments that launch a child in this posture.
    pub(crate) fn args(self) -> [&'static str; 2] {
        [self.flag(), POSTURE]
    }
}

/// What one binary's `--help` says about the arguments the harness passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dialect {
    pub(crate) posture: Posture,
    /// Whether the binary documents `--plain`, the flag that asks for the line
    /// front on a terminal. The control predates it; the line measures run on a
    /// pipe, so neither arm needs the flag to be driven.
    pub(crate) plain: bool,
}

impl Dialect {
    /// Reads `bin`'s own `--help` — once per binary per invocation — and panics
    /// with the child's own words when the binary will not say.
    pub(crate) fn of(bin: &Path) -> Self {
        let output = Command::new(bin)
            .arg("--help")
            .output()
            .unwrap_or_else(|error| panic!("{} would not run: {error}", bin.display()));
        let mut help = String::from_utf8_lossy(&output.stdout).into_owned();
        help.push_str(&String::from_utf8_lossy(&output.stderr));
        Self::detect(&help)
            .unwrap_or_else(|error| panic!("{}: {error}; help was:\n{help}", bin.display()))
    }

    /// The dialect `help` documents: `--mode` when the binary states it, else
    /// `--approval-mode`. A binary that states neither cannot be launched in
    /// the run's posture at all, which is an error rather than a guess.
    pub(crate) fn detect(help: &str) -> Result<Self, String> {
        let posture = if documents(help, "--mode") {
            Posture::Mode
        } else if documents(help, "--approval-mode") {
            Posture::ApprovalMode
        } else {
            return Err(
                "the help names neither `--mode` nor `--approval-mode`, so the \
                 read-only posture a perf run launches in has no spelling here"
                    .to_owned(),
            );
        };
        Ok(Self {
            posture,
            plain: documents(help, "--plain"),
        })
    }
}

/// Whether `help` lists `flag` as an option of its own: the name starts a line
/// (after indentation) and ends there or runs into its value, so `--mode` does
/// not match `--approval-mode`, `--permission-mode`, or a mention of it inside
/// another option's description.
fn documents(help: &str, flag: &str) -> bool {
    help.lines().any(|line| {
        line.trim_start()
            .strip_prefix(flag)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '<', ',']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The candidate's own help, trimmed to the options that matter here.
    const CANDIDATE_HELP: &str = "\
Usage: saya [OPTIONS] [COMMAND]

Options:
      --workspace <DIR>
          Bind the session workspace root to this directory (default: the git worktree top)
      --mode <MODE>
          Start in this consent mode — ask|accept-edits|plan|bypass| read-only|never
      --permission-mode <MODE>
          Deprecated alias of `--mode`, removed next release; plan is interactive only
      --approval-mode <MODE>
          Deprecated alias of `--mode`, removed next release: ask|read-only|never|bypass
      --plain
          Run the plain line REPL on a terminal instead of the full-screen TUI
";

    /// The control's help, trimmed the same way: it states `--approval-mode`
    /// as its own option and has no `--mode` and no `--plain`.
    const CONTROL_HELP: &str = "\
Usage: saya [OPTIONS] [COMMAND]

Options:
      --workspace <DIR>
          Bind the session workspace root to this directory (default: the git worktree top)
      --permission-mode <MODE>
          Start in this permission mode: ask|accept-edits|plan|bypass (session only)
      --approval-mode <MODE>
          When tool calls need approval: ask|read-only|never|bypass (headless default: never, no `shell`)
";

    #[test]
    fn a_binary_that_states_mode_launches_in_mode() {
        let dialect = Dialect::detect(CANDIDATE_HELP).expect("the candidate names a posture flag");
        assert_eq!(dialect.posture, Posture::Mode);
        assert_eq!(dialect.posture.args(), ["--mode", "read-only"]);
        assert!(dialect.plain, "the candidate documents `--plain`");
    }

    #[test]
    fn a_control_that_only_states_approval_mode_launches_with_that_flag() {
        let dialect = Dialect::detect(CONTROL_HELP).expect("the control names a posture flag");
        assert_eq!(dialect.posture, Posture::ApprovalMode);
        assert_eq!(
            dialect.posture.args(),
            ["--approval-mode", "read-only"],
            "the control rejects `--mode`; the same posture needs its own spelling"
        );
        assert!(!dialect.plain, "the control predates `--plain`");
    }

    #[test]
    fn a_binary_that_states_neither_spelling_is_an_error() {
        let error = Dialect::detect("Usage: saya [OPTIONS]\n\nOptions:\n      --workspace <DIR>\n")
            .expect_err("no posture flag means no way to launch the run's posture");
        assert!(
            error.contains("--approval-mode"),
            "the error names the flags: {error}"
        );
    }

    #[test]
    fn a_flag_mentioned_inside_another_options_text_is_not_an_option() {
        let help = "\
Options:
      --permission-mode <MODE>
          Deprecated alias of `--mode`, removed next release
";
        assert!(
            Dialect::detect(help).is_err(),
            "`--mode` inside another option's description is not a spelling of the posture"
        );
    }

    #[test]
    fn both_arms_state_the_same_posture() {
        let control = Dialect::detect(CONTROL_HELP).unwrap();
        let candidate = Dialect::detect(CANDIDATE_HELP).unwrap();
        assert_eq!(
            control.posture.args()[1..],
            candidate.posture.args()[1..],
            "only the flag's name may differ between the arms"
        );
    }
}
