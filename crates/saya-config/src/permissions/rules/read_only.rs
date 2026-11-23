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
//! ([`catalogue`]), why a pager, which needs a terminal this capture never is,
//! is refused outright, and why a `git` spelling is read against the
//! repository's own configuration ([`repository`]).
//!
//! A command is a statement about what a program *does*; the set is a claim
//! about what it *reads*. The two meet on the command line: a vetted command
//! names only paths inside the workspace that the workspace tools would also
//! read ([`paths`]), carries no expansion or unquoted glob the shell would
//! perform, and reads no tree of hidden files.

use std::path::{Path, PathBuf};

use super::resolve::resolve;
use super::scan::{Scan, scan, tokens};

mod catalogue;
mod paths;
mod repository;

use catalogue::{GIT, SET, git_dir, git_qualifies};

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

/// Where a command's own `git` would run, as far as the command line states
/// it: the workspace root, and the leading `cd` the scanner stripped. Both are
/// the command's own words, so a vetted `git` is read against the repository
/// those words point at rather than against an assumed one.
#[derive(Clone, Copy)]
pub(super) struct WorkingDir<'a> {
    root: Option<&'a Path>,
    cd: Option<&'a str>,
}

impl<'a> WorkingDir<'a> {
    pub(super) fn new(root: Option<&'a Path>, cd: Option<&'a str>) -> Self {
        Self { root, cd }
    }

    pub(super) fn root(self) -> Option<&'a Path> {
        self.root
    }

    /// The directory a `git` segment given `-C <git_dir>` starts in, or `None`
    /// where the command line does not place it inside the workspace.
    pub(super) fn starts_in(self, git_dir: Option<&str>) -> Option<PathBuf> {
        let mut path = self.root?.to_path_buf();
        for part in self.cd.into_iter().chain(git_dir) {
            // A rooted or traversal-carrying directory is not placed here: the
            // path rule already asks for one, and this refuses to guess.
            if part.starts_with('/') || part.split('/').any(|component| component == "..") {
                return None;
            }
            path = path.join(part);
        }
        Some(path)
    }
}

/// Whether every part of the command only reads, over the workspace `dir`
/// states.
pub(super) fn qualifies(scan: &Scan, dir: WorkingDir<'_>) -> bool {
    // The shapes that make the written command not the command that runs: a
    // substitution, an expansion, an unquoted glob, an unbalanced quote, an
    // unsafe redirect.
    if scan.substitution || scan.unbalanced || scan.unsafe_redirect || scan.expansion || scan.glob {
        return false;
    }
    scan.segments
        .iter()
        .all(|segment| segment_qualifies(segment, dir))
}

/// Every name the set would read as a path in `command`, in the spelling the
/// command gives it.
///
/// The set's own path rule is [`crate::sensitive::sensitive_path`] — the one
/// list a configuration crate can state. A front that can also see the
/// workspace's private-path screen (the redactor, the session store) asks this
/// for the names a command carries, and refuses the question-skipping answer for
/// one of them. It is the same reading the decision itself uses
/// ([`paths::spellings`] over every segment's arguments), never a second list.
pub fn named_paths(command: &str) -> Vec<String> {
    scan(command)
        .segments
        .iter()
        .flat_map(|segment| {
            // The program is not a path: the arguments are what a vetted command
            // may read.
            let arguments = tokens(segment);
            arguments
                .into_iter()
                .skip(1)
                .flat_map(|argument| {
                    paths::spellings(&argument)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether one segment only reads: the program it really runs is the program
/// written, that program is vetted with these arguments, every path the
/// arguments name is one the workspace tools would also read, and — for `git`
/// — the repository's own configuration runs nothing.
fn segment_qualifies(segment: &str, dir: WorkingDir<'_>) -> bool {
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
    if !paths::vetted(arguments) {
        return false;
    }
    if program == GIT {
        return git_qualifies(arguments)
            && !repository::runs_programs(dir, git_dir(arguments).as_deref());
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
    /// Option-and-value pairs refused together, for a value that changes what
    /// the program reads rather than what it prints (`grep -d recurse`).
    values: &'static [(&'static str, &'static str)],
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
            values: &[],
        }
    }

    /// Refuses the pairs given: an option and the one value of it that puts
    /// the program back in the question.
    const fn refusing_values(mut self, values: &'static [(&'static str, &'static str)]) -> Self {
        self.values = values;
        self
    }

    /// A program whose every argument only describes what to print.
    const fn plain(program: &'static str) -> Self {
        Self::new(program, &[], &[], usize::MAX)
    }

    fn qualifies(&self, arguments: &[String]) -> bool {
        let mut positionals = 0;
        for (index, argument) in arguments.iter().enumerate() {
            if self
                .values
                .iter()
                .any(|(option, value)| argument == option && follows(arguments, index, value))
            {
                return false;
            }
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

/// Whether the argument after `index` is `value`.
fn follows(arguments: &[String], index: usize, value: &str) -> bool {
    arguments.get(index + 1).is_some_and(|next| next == value)
}
