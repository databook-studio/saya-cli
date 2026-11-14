//! Read-only inspection commands run through `shell`: `cat`, `head`, `tail`,
//! `sed -n`, `ls`, `find`, `grep`, and `rg`. They are classified with the read
//! tools' semantics — a first look at a target is progress, a repeat of the
//! same target with the same result is not, and a look that finds nothing
//! moved nothing — so using the shell to inspect a file cannot hide churn from
//! the progress guard. A file inspection is keyed by its arguments as well as
//! its path, so paging a large file by range or re-tailing a growing log is a
//! new look, not a repeat.

/// The read-tool family a shell inspection command stands in for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Read,
    Search,
    List,
    Glob,
}

impl Family {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Search => "search",
            Self::List => "list",
            Self::Glob => "glob",
        }
    }
}

/// One parsed inspection command: the family and the target it looks at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Inspection {
    family: Family,
    target: String,
}

impl Inspection {
    /// The key the seen-target set remembers it under.
    pub(super) fn key(&self) -> String {
        format!(
            "shell-inspect\u{0}{}\u{0}{}",
            self.family.as_str(),
            self.target
        )
    }
}

/// Parses a shell command as an inspection command, or `None` when it is not
/// one: a build, a test run, an in-place `sed`, a following `tail`, or a
/// command whose first program is not an inspection tool.
pub(super) fn parse(command: &str) -> Option<Inspection> {
    let command = strip_cd_prefix(command);
    let mut words = command.split_whitespace();
    let program = words.next()?;
    let program = program.rsplit('/').next().unwrap_or(program);
    let args: Vec<&str> = words.collect();
    let (family, target) = match program {
        // A file read is keyed by every argument, so `head -n 20 f` and
        // `head -n 40 f`, or two `sed -n` ranges, are different looks.
        "cat" | "head" => (Family::Read, arguments(&args)),
        "tail" => {
            if follows(&args) {
                return None;
            }
            (Family::Read, arguments(&args))
        }
        // `sed -n` only reads; an in-place `sed` changes the workspace and must
        // not be classified as an inspection.
        "sed" if quiet(&args) => (Family::Read, arguments(&args)),
        "ls" => (Family::List, targets(&args)),
        "find" => (Family::Glob, targets(&args)),
        "grep" | "rg" => (Family::Search, targets(&args)),
        _ => return None,
    };
    Some(Inspection { family, target })
}

/// Drops a leading `cd DIR &&`, so `cd x && sed -n ...` is the same read as
/// the bare command. Only the `&&` form is a prefix; anything else is left to
/// the generic shell rule.
fn strip_cd_prefix(command: &str) -> &str {
    let mut rest = command.trim_start();
    while let Some(after) = rest.strip_prefix("cd ")
        && let Some((_dir, tail)) = after.split_once("&&")
    {
        rest = tail.trim_start();
    }
    rest
}

/// The arguments as written, joined: for a file read this carries the range
/// or script and the path together.
fn arguments(args: &[&str]) -> String {
    args.join(" ")
}

/// Whether `tail` was asked to follow the file (`-f`, `-F`, `--follow`). A
/// following tail never ends, so it is not a read the classifier can judge.
fn follows(args: &[&str]) -> bool {
    args.iter().any(|arg| {
        if arg.starts_with("--") {
            return *arg == "--follow" || arg.starts_with("--follow=");
        }
        arg.starts_with('-') && (arg.contains('f') || arg.contains('F'))
    })
}

/// The non-flag arguments joined: the search pattern and scope, or the
/// directories a list or glob looks at.
fn targets(args: &[&str]) -> String {
    args.iter()
        .filter(|arg| !arg.starts_with('-'))
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `sed` runs quiet, so it only prints the lines it is told to. A
/// short-flag cluster that carries `-i` is in-place editing, not an
/// inspection, so it is refused however the cluster is spelled.
fn quiet(args: &[&str]) -> bool {
    args.iter().any(|arg| {
        if arg.starts_with("--") {
            return *arg == "--quiet";
        }
        arg.starts_with('-') && arg.contains('n') && !arg.contains('i')
    })
}
