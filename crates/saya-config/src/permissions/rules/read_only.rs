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
//! - the program is one of [`SET`], or `git` given a vetted subcommand
//!   ([`git_qualifies`]), and no argument it is given writes, follows, or runs
//!   a program;
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
//! only `git` spellings that cannot page, exec, or write are vetted, and why a
//! pager, which needs a terminal this capture never is, is refused outright.

use super::resolve::resolve;
use super::scan::{Scan, tokens};

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
    /// Letters refused inside a clustered short option: `-of`, `-T`, `-F`.
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

/// `git`'s subcommand decides, so it is read separately from [`SET`].
const GIT: &str = "git";

/// The global options `git` may be given before its subcommand: the directory
/// it changes to is taken with its value, and the rest only remove work.
const GIT_GLOBALS: &[&str] = &[
    "--no-pager",
    "--no-optional-locks",
    "--no-replace-objects",
    "--literal-pathspecs",
];

/// The spellings no vetted `git` subcommand may carry: they run another
/// program (`--ext-diff`, `--textconv`, a pager), write a file (`--output`),
/// or point `git` at one (`--exec-path`, `--config-env`).
const GIT_REFUSES: &[&str] = &[
    "--ext-diff",
    "--textconv",
    "--output",
    "--open-files-in-pager",
    "--exec-path",
    "--config-env",
    "--paginate",
];

/// Whether a `git` command only reads: a vetted subcommand, and no argument
/// that would run a program or write a file.
fn git_qualifies(arguments: &[String]) -> bool {
    let mut index = 0;
    while let Some(token) = arguments.get(index) {
        if !token.starts_with('-') {
            break;
        }
        // `-C <dir>` changes where `git` reads, and changes nothing else. A
        // missing directory is not a subcommand, so the read stops here.
        if token == "-C" {
            index += 2;
            continue;
        }
        if !GIT_GLOBALS.contains(&token.as_str()) {
            return false;
        }
        index += 1;
    }
    let Some(subcommand) = arguments.get(index) else {
        return false;
    };
    let rest = &arguments[index + 1..];
    if rest.iter().any(|argument| {
        GIT_REFUSES
            .iter()
            .any(|spelling| argument.starts_with(spelling))
    }) {
        return false;
    }
    match subcommand.as_str() {
        "status" | "log" | "diff" | "show" | "rev-parse" | "ls-files" | "blame" => true,
        "branch" => branch_qualifies(rest),
        // `remote` reads only as a listing, which is what `-v` asks for: the
        // subcommands that add, rename, or remove a remote are not vetted.
        "remote" => {
            !rest.is_empty()
                && rest
                    .iter()
                    .all(|argument| argument == "-v" || argument == "--verbose")
        }
        _ => false,
    }
}

/// `git branch` reads only as a listing: no arguments, a listing flag, or a
/// pattern after `--list`. A bare word names a branch to create or delete.
fn branch_qualifies(arguments: &[String]) -> bool {
    let mut listing = false;
    for argument in arguments {
        match argument.as_str() {
            "-a" | "--all" | "-r" | "--remotes" => {}
            "--list" => listing = true,
            _ if argument.starts_with('-') => return false,
            _ if listing => {}
            _ => return false,
        }
    }
    true
}

/// The vetted programs. Every entry names the arguments that would take it out
/// of the set, so a new program arrives with its own read-only argument check.
const SET: &[Vetted] = &[
    Vetted::plain("cat"),
    Vetted::plain("cut"),
    Vetted::plain("df"),
    Vetted::plain("diff"),
    Vetted::plain("du"),
    Vetted::plain("echo"),
    Vetted::plain("grep"),
    Vetted::plain("head"),
    Vetted::plain("ls"),
    Vetted::plain("pwd"),
    Vetted::plain("stat"),
    Vetted::plain("type"),
    Vetted::plain("wc"),
    Vetted::plain("which"),
    // `file -C` compiles a magic database into the user's tree.
    Vetted::new("file", &["-C", "--compile"], &['C'], usize::MAX),
    // `tail --follow` never returns: the capture would wait on it.
    Vetted::new("tail", &["--follow"], &['f', 'F'], usize::MAX),
    // `sort -o`/`--output` writes the sorted file, and `-T` its temporaries.
    Vetted::new(
        "sort",
        &[
            "--output",
            "--compress-program",
            "--temporary-directory",
            "--random-source",
        ],
        &['o', 'T'],
        usize::MAX,
    ),
    // `uniq in out` writes `out`.
    Vetted::new("uniq", &[], &[], 1),
    // `tree -o`/`--output` writes the tree to a file.
    Vetted::new("tree", &["--output"], &['o'], usize::MAX),
    // `find`'s actions run a program, delete a path, or write a file.
    Vetted::new(
        "find",
        &[
            "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf",
            "-fls",
        ],
        &[],
        usize::MAX,
    ),
    // `rg --pre` runs a program over every file it reads.
    Vetted::new("rg", &["--pre", "--pre-glob", "--pager"], &[], usize::MAX),
];
