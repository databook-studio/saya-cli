//! The launchers the deny side reads through: the programs that run another
//! program named after them, and the interpreters whose inline code the reading
//! cannot see.
//!
//! Reading a launcher is what keeps a deny rule honest. `timeout 5 curl`,
//! `nice -n 5 curl`, `nohup curl`, `stdbuf -oL curl`, `time curl`,
//! `xargs -I{} curl`, `find . -exec curl {} \;`, and `eval "curl x"` all name
//! `curl` exactly as `curl x` does: a wrapper's options, the values of those
//! options, and its terminator are not the program. An interpreter's `-c`/`-e`
//! program is code, not a command line this reading can see, so it says so
//! ([`Nested::Unreadable`]) rather than guessing — never claiming a command
//! runs what it did not read.

/// What a segment hands the deny side beyond itself.
pub(super) enum Nested {
    /// Nothing: the segment runs its own program.
    None,
    /// Command lines to read one level deeper: a shell's or `eval`'s script, or
    /// the command line of a `find` action.
    Segments(Vec<String>),
    /// Code this reading cannot see, so the command cannot be decided from it.
    Unreadable,
}

/// The command lines a resolved segment runs besides itself, when it runs one.
pub(super) fn nested(program: &str, arguments: &[String]) -> Nested {
    match program {
        "eval" => script(arguments),
        "find" => Nested::Segments(actions(arguments)),
        "sh" | "bash" | "zsh" | "dash" => shell_script(arguments),
        _ if runs_unreadable_code(program, arguments) => Nested::Unreadable,
        _ => Nested::None,
    }
}

/// How many tokens the wrapper at the front of a segment consumes before the
/// program it runs begins — the wrapper token included. `None` when the front
/// token runs no later program, and when nothing follows it (`env` alone is
/// still `env`, a program in its own right).
pub(super) fn wrapper_skip(tokens: &[String]) -> Option<usize> {
    let (wrapper, rest) = tokens.split_first()?;
    let skip = match wrapper.as_str() {
        // `timeout` waits out its duration before it runs anything.
        "timeout" => 2 + option_count(rest, TIMEOUT),
        // `env` sets the environment; `exec` replaces the shell with the
        // program; `nice`, `stdbuf`, and `time` adjust how it runs; `nohup`
        // outlives the hangup; `xargs` builds the command line from its input;
        // `command` and `builtin` bypass a function of the same name: each runs
        // what follows it.
        "env" => 1 + option_count(rest, ENV),
        "exec" => 1 + option_count(rest, EXEC),
        "nice" => 1 + option_count(rest, NICE),
        "stdbuf" => 1 + option_count(rest, STDBUF),
        "time" => 1 + option_count(rest, TIME),
        "xargs" => 1 + option_count(rest, XARGS),
        "command" | "builtin" | "nohup" => 1 + option_count(rest, &[]),
        _ => return None,
    };
    (skip < tokens.len()).then_some(skip)
}

/// The leading options of a wrapped program, with the value of an option that
/// carries one — never the program. The sets are per wrapper because one letter
/// is not one option to two of them: `env -i` takes no value, while
/// `stdbuf -i 0` does.
fn option_count(tokens: &[String], valued: &[&str]) -> usize {
    let mut skipped = 0;
    while let Some(token) = tokens.get(skipped) {
        if token == "--" {
            return skipped + 1;
        }
        if !is_option(token) {
            break;
        }
        skipped += 1;
        if valued.contains(&token.as_str()) && tokens.get(skipped).is_some() {
            skipped += 1;
        }
    }
    skipped
}

fn is_option(token: &str) -> bool {
    token.starts_with('-') && token.len() > 1
}

/// The options whose value is the next token, so a value is never read as the
/// program: `env -u NAME`, `exec -a NAME`, `nice -n 5`, `timeout -s SIGNAL`,
/// `stdbuf -o 0`, `xargs -n 1`. The attached spellings (`-oL`, `-I{}`, `-n1`,
/// `--signal=KILL`) carry their value in their own token.
const ENV: &[&str] = &[
    "-u",
    "-C",
    "-S",
    "-a",
    "--unset",
    "--chdir",
    "--split-string",
    "--argv0",
];
const EXEC: &[&str] = &["-a", "--argv0"];
const NICE: &[&str] = &["-n", "--adjustment"];
const TIME: &[&str] = &["-f", "-o", "--format", "--output"];
const TIMEOUT: &[&str] = &["-s", "-k", "--signal", "--kill-after"];
const STDBUF: &[&str] = &["-i", "-o", "-e", "--input", "--output", "--error"];
const XARGS: &[&str] = &[
    "-I",
    "-n",
    "-P",
    "-L",
    "-s",
    "-E",
    "-d",
    "-a",
    "--arg-file",
    "--delimiter",
    "--max-args",
    "--max-chars",
    "--max-lines",
    "--max-procs",
    "--process-slot-var",
    "--eof",
];

/// `eval`'s arguments, joined, are one script — read with the same reading as
/// any shell's.
fn script(arguments: &[String]) -> Nested {
    if arguments.is_empty() {
        return Nested::None;
    }
    Nested::Segments(vec![arguments.join(" ")])
}

/// The `-c` script a shell runs: `sh -c 'x'`, `bash -lc 'x'`, `zsh -c "x"`,
/// `dash -c 'x'`. The script runs to the end of the segment, so everything
/// after the flag is part of what the shell reads.
fn shell_script(arguments: &[String]) -> Nested {
    let Some(flag) = arguments
        .iter()
        .position(|argument| runs_a_script(argument))
    else {
        return Nested::None;
    };
    script(&arguments[flag + 1..])
}

/// Whether this argument is a shell's `-c`, or a cluster holding it (`-lc`,
/// `-ec`): the next token is the script the shell runs.
fn runs_a_script(argument: &str) -> bool {
    argument.starts_with('-') && !argument.starts_with("--") && argument.contains('c')
}

/// The programs a `find` runs: the command line of every `-exec`, `-execdir`,
/// and `-ok` action, up to its terminator (`\;` or `+`). A `find` that runs
/// nothing is a search, and names no program.
fn actions(arguments: &[String]) -> Vec<String> {
    let mut programs = Vec::new();
    let mut pending = arguments;
    while let Some(action) = pending.iter().position(|argument| is_action(argument)) {
        let command = &pending[action + 1..];
        let end = command
            .iter()
            .position(|argument| is_terminator(argument))
            .unwrap_or(command.len());
        if end > 0 {
            programs.push(command[..end].join(" "));
        }
        if end == command.len() {
            break;
        }
        pending = &command[end + 1..];
    }
    programs
}

fn is_action(argument: &str) -> bool {
    matches!(argument, "-exec" | "-execdir" | "-ok" | "-okdir")
}

fn is_terminator(argument: &str) -> bool {
    matches!(argument, "\\;" | ";" | "+")
}

/// Whether this program is an interpreter told its code inline, which this
/// reading cannot see. `awk -f prog.awk` is a file it does not read either.
fn runs_unreadable_code(program: &str, arguments: &[String]) -> bool {
    match program {
        "awk" | "gawk" => arguments.iter().any(|argument| !is_option(argument)),
        "python" | "python3" | "perl" | "ruby" | "node" | "php" => arguments
            .iter()
            .any(|argument| is_inline_code_flag(argument)),
        _ => false,
    }
}

/// The flags that make an interpreter's next argument (or its own attached
/// text) code: `python3 -c`, `perl -e`, `ruby -e`, `node -e`, `php -r`.
fn is_inline_code_flag(argument: &str) -> bool {
    matches!(argument, "-c" | "-e" | "-r" | "--eval" | "--exec" | "--run")
}
