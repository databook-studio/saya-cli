//! The vetted read-only set: the commands whose every part only reads, so an
//! asking mode can run them without putting the question to the operator.
//!
//! The set is decided by *parsing* the command with the same reading the deny
//! side uses ([`super::scan`] and [`super::resolve`]) — never by a prefix or a
//! substring match. A command qualifies only when every segment does:
//!
//! - every segment resolves to the program it really runs, unchanged by the
//!   reading: a leading `NAME=value`, a wrapper (`env`, `xargs`, `timeout`,
//!   `sudo`, …), a subshell opener, or a path would be stripped or rewritten,
//!   so a segment the reading changes keeps asking;
//! - the program is one of the catalogue's set, or `git` given a vetted
//!   subcommand (see [`catalogue`]), and no argument it is given
//!   writes, follows, or runs a program;
//! - nothing in the command is a command or process substitution, a heredoc,
//!   an unbalanced quote, or an output redirect to anything but an fd
//!   duplication or `/dev/null`. A pipeline or a chain of `&&`, `||`, `;`
//!   qualifies because every one of its segments does.
//!
//! This skips a question; it does not widen what a shell may reach. A host
//! call still runs under the lane rules, a sandboxed call still runs in the
//! sandbox, the deny list still refuses first, and the modes that decide for
//! themselves (`bypass`, `read-only`, `never`) are read exactly as before.
//!
//! The set is about the command line, not about what a program's own
//! configuration makes it do: a repository's `git` configuration can name a
//! filter, exactly as a `make` target names its own commands — which is why
//! only `git` spellings that cannot page, exec, or write are vetted
//! ([`catalogue`]), and why a pager, which needs a terminal this capture never
//! is, is refused outright.

use super::resolve::resolve;
use super::scan::{Scan, tokens};

mod catalogue;

use catalogue::{GIT, SET, git_qualifies};

/// The switch that offers the set: `[permissions] read_only_commands`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReadOnlySet {
    /// The vetted read-only commands run without asking. The default.
    #[default]
    On,
    /// Nothing is vetted: every command is put to the operator as it was
    /// before the set existed.
    Off,
}

impl ReadOnlySet {
    /// Whether the set may answer for a command.
    pub(super) fn permits(self) -> bool {
        matches!(self, Self::On)
    }
}

/// Whether every part of the command only reads.
pub(super) fn qualifies(scan: &Scan) -> bool {
    // The shapes that make the written command not the command that runs.
    if scan.substitution || scan.unbalanced || scan.unsafe_redirect {
        return false;
    }
    scan.segments
        .iter()
        .all(|segment| segment_qualifies(segment))
}

/// Whether one segment only reads: the program it really runs is the program
/// written, and that program is vetted with these arguments.
fn segment_qualifies(segment: &str) -> bool {
    let raw = tokens(segment);
    if raw.is_empty() {
        return false;
    }
    // The deny side's own reading. Whatever it changes — an assignment, a
    // wrapper, an opener, a path — is a program the segment does not name.
    if resolve(raw.clone()) != raw {
        return false;
    }
    let Some((program, arguments)) = raw.split_first() else {
        return false;
    };
    if program == GIT {
        return git_qualifies(arguments);
    }
    SET.iter()
        .find(|vetted| vetted.program == program)
        .is_some_and(|vetted| vetted.qualifies(arguments))
}

/// One vetted program, with the arguments that put it outside the set.
struct Vetted {
    program: &'static str,
    /// Option spellings refused at the front of an option token: `--output`,
    /// `--output=file`, `-exec`, `-execdir`.
    refuses: &'static [&'static str],
    /// Letters refused inside a clustered short option: `-of`, `-T`, `-C`.
    letters: &'static [char],
    /// Positional arguments beyond this many write a file of their own
    /// (`uniq in out`).
    positionals: usize,
}

impl Vetted {
    const fn new(
        program: &'static str,
        refuses: &'static [&'static str],
        letters: &'static [char],
        positionals: usize,
    ) -> Self {
        Self {
            program,
            refuses,
            letters,
            positionals,
        }
    }

    /// A program whose every argument only describes what to print.
    const fn plain(program: &'static str) -> Self {
        Self::new(program, &[], &[], usize::MAX)
    }

    fn qualifies(&self, arguments: &[String]) -> bool {
        let mut positionals = 0;
        for argument in arguments {
            if argument.len() > 1 && argument.starts_with('-') {
                if self
                    .refuses
                    .iter()
                    .any(|spelling| argument.starts_with(spelling))
                {
                    return false;
                }
                let cluster = &argument[1..];
                if !argument.starts_with("--")
                    && self.letters.iter().any(|letter| cluster.contains(*letter))
                {
                    return false;
                }
            } else {
                positionals += 1;
                if positionals > self.positionals {
                    return false;
                }
            }
        }
        true
    }
}
