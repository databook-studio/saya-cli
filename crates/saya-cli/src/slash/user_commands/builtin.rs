// The commands every session starts with. They are ordinary user commands in
// every way that matters: the same body, the same `$ARGUMENTS` expansion, and
// the same route through the agent as a message — so there is one operation
// behind the built-in and a user's own file. A user or repository command with
// the same name replaces one; the built-in is only the default.

/// The built-in commands, in name order. Each is short because it is a
/// prompt, and every sentence is a rule the agent is expected to follow.
const BUILTIN_COMMANDS: &[(&str, &str, &str)] = &[
    (
        "commit",
        "Commit the current task's changes with a conventional message",
        "Commit the current task's changes.\n\n\
         1. Run `git status` and `git diff` (unstaged and staged) and read what changed.\n\
         2. Stage only the files this task changed. Never `git add -A` or `git add .`, and \
         never stage unrelated files.\n\
         3. Write a conventional-commit message (`feat:`, `fix:`, `docs:`, ...) for the staged \
         change. Use this hint if one was given: $ARGUMENTS\n\
         4. Commit. Never pass `--no-verify`, never amend a commit that was already pushed, and \
         never commit secrets or credential files. Do not add a co-author trailer.",
    ),
    (
        "pr",
        "Push the current branch and open a pull request",
        "Open a pull request for the current branch.\n\n\
         1. Inspect `git status`, `git log`, and the diff against the default branch.\n\
         2. Push the current branch only if your permission mode approves that shell call. Never \
         force-push, and never push to or target the default branch.\n\
         3. If `gh` is installed, run `gh pr create` with a clear title and body. Otherwise \
         print the repository's compare URL for this branch.\n\
         4. $ARGUMENTS",
    ),
];

/// Whether `name` is one of the shipped defaults.
fn is_builtin_name(name: &str) -> bool {
    BUILTIN_COMMANDS.iter().any(|(builtin, ..)| *builtin == name)
}

/// Seeds the defaults into a freshly built set. A built-in carries no real
/// path: it was not read from a file, and the help text says so.
fn seed_builtins(commands: &mut UserCommands) {
    for (name, description, body) in BUILTIN_COMMANDS {
        commands.entries.insert(
            (*name).to_string(),
            Entry::Ready(UserCommand {
                name: (*name).to_string(),
                description: Some((*description).to_string()),
                body: (*body).to_string(),
                source: CommandSource::Builtin,
                path: PathBuf::from(format!("<built-in>/{name}.md")),
            }),
        );
    }
}
