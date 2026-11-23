//! The catalogue: the programs the read-only set vets, and the arguments that
//! put each of them back into the question.
//!
//! A program's own configuration can make it run something the command line
//! never names — a repository's `git` can name a diff filter, a pager, or an
//! external program — so only `git` spellings that cannot page, exec, or write
//! are vetted here, and every other program is listed with the arguments that
//! would take it out of the set.

use super::Vetted;

/// `git`'s subcommand decides, so it is read separately from [`SET`].
pub(super) const GIT: &str = "git";

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
pub(super) fn git_qualifies(arguments: &[String]) -> bool {
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
pub(super) const SET: &[Vetted] = &[
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
    // `file -C` compiles a magic database into the user's tree, in a cluster
    // (`-bC`) as much as at the front of the token.
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
