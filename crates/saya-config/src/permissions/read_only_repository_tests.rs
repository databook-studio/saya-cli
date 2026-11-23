//! The repository's own configuration: what a vetted `git` can be made to run.
//!
//! A repository's local configuration names programs `git` runs on its behalf —
//! a filesystem monitor on `status`, an external diff or a textconv driver on
//! `diff`, `log`, and `show`, a filter's `process`, an ssh command. Saya's
//! tools can write `.git/config` (`crates/saya-cli/src/agent/tools/workspace/
//! write.rs` does not special-case `.git`, and the shell may write it in either
//! lane), and the model authored files are not trusted, so a `git` spelling is
//! vetted only over a configuration that names no program — and otherwise the
//! question is kept.

use super::read_only_fixture::Workspace;
use super::{CommandDecision, CommandRules, decide};

/// Whether the default table reads this command as a vetted read-only one.
fn vetted(workspace: &Workspace, command: &str) -> bool {
    decide(&CommandRules::default(), command, Some(workspace.path())) == CommandDecision::ReadOnly
}

/// Every key that makes a vetted `git` subcommand run a program of the
/// repository's choosing keeps the question.
#[test]
fn a_repository_that_runs_programs_keeps_the_question() {
    let configurations = [
        // `git status` runs it.
        "[core]\n\tfsmonitor = ./hook.sh\n",
        // `git log`/`diff`/`show` page through it.
        "[core]\n\tpager = ./hook.sh\n",
        "[core]\n\tsshCommand = ./hook.sh\n",
        // `git diff`/`log -p`/`show` run it instead of diffing.
        "[diff]\n\texternal = ./hook.sh\n",
        "[diff \"driver\"]\n\ttextconv = ./hook.sh\n",
        // A clean/smudge filter runs it over the files `git diff` reads.
        "[filter \"driver\"]\n\tprocess = ./hook.sh\n",
        // The configuration names a file of its own to read the rest from.
        "[include]\n\tpath = ./extra.config\n",
        "[includeIf \"gitdir:~/x/\"]\n\tpath = ./extra.config\n",
    ];
    for configuration in configurations {
        let workspace = Workspace::with_config("programs", configuration);
        for command in ["git status", "git log -5", "git diff", "git show HEAD"] {
            assert!(
                !vetted(&workspace, command),
                "`{command}` must ask over the configuration {configuration:?}"
            );
        }
    }
}

/// A repository whose configuration names no program is vetted as before: the
/// refusal is about what the configuration runs, never about `git` itself.
#[test]
fn a_repository_that_runs_nothing_is_vetted() {
    let workspace = Workspace::with_config(
        "benign",
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\
         [branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n\
         [diff \"driver\"]\n\ttextconv = \n",
    );
    for command in ["git status", "git log -5", "git diff", "git show HEAD"] {
        assert!(
            vetted(&workspace, command),
            "`{command}` only reads a repository that runs nothing"
        );
    }
}

/// A repository this bound cannot read is not vetted: no repository inside the
/// workspace (so the one `git` would find is outside it), and a `.git` that is
/// a pointer file rather than the configuration itself.
#[test]
fn a_repository_that_cannot_be_read_keeps_the_question() {
    let bare = Workspace::bare("unknown");
    assert!(
        !vetted(&bare, "git status"),
        "a workspace with no repository runs against one this bound cannot read"
    );
    let pointer = Workspace::with_config("pointer", "[core]\n\trepositoryformatversion = 0\n");
    std::fs::remove_dir_all(pointer.path().join(".git")).expect("the fixture holds a .git");
    std::fs::write(
        pointer.path().join(".git"),
        "gitdir: /elsewhere/.git/worktrees/x\n",
    )
    .expect("the pointer must be writable");
    assert!(
        !vetted(&pointer, "git status"),
        "a `.git` pointer names a configuration outside the workspace"
    );
}

/// A directory the command changes to before `git` runs is read where it
/// points: a nested repository's own configuration decides, whether the
/// command states it with a leading `cd` or with `git -C`.
#[test]
fn a_nested_repository_is_read_where_the_command_goes() {
    let benign = Workspace::new("nested-clean");
    benign.nested("sub", "[core]\n\trepositoryformatversion = 0\n");
    for command in ["cd sub && git status", "git -C sub status"] {
        assert!(
            vetted(&benign, command),
            "`{command}` reads a nested repository that runs nothing"
        );
    }
    let running = Workspace::new("nested-running");
    running.nested("sub", "[core]\n\tfsmonitor = ./hook.sh\n");
    for command in ["cd sub && git status", "git -C sub status"] {
        assert!(
            !vetted(&running, command),
            "`{command}` must ask over a nested repository that runs a program"
        );
    }
}
