//! `shell`'s fact lines: the command and the working directory the operator is
//! being asked to approve, and the route the call takes.
//!
//! The command is model-authored, so it is put through the shell's own
//! sanitiser ([`sanitise_prompt_text`](crate::agent::tools::shell_policy::sanitise_prompt_text))
//! before it reaches the prompt: a raw ESC or CR could rewrite the line being
//! read, and an invisible format character could hide part of what is being
//! approved. The schema admits a 64 KiB command, so the echo is bounded: the
//! prompt paints a head and a tail, and between them names a private file
//! holding the exact bytes that will run. The answer is bound to that file;
//! when it cannot be written, the whole command is shown instead, never a
//! truncated one.

use std::path::Path;

use serde_json::Value;

use crate::agent::tools::shell_policy::{
    MAX_COMMAND_BYTES, MAX_TIMEOUT_MILLIS, Operation, operation, sanitise_prompt_text,
};

use super::{ApprovalFacts, body, command_file};

/// The most characters of the command the prompt paints as its head. The
/// schema admits a 64 KiB command; a prompt that echoed it whole would flood
/// the operator's terminal, so the echo is cut and names the saved file.
const MAX_PROMPT_COMMAND_CHARS: usize = 2_048;
/// The most lines the head paints, so a command of newlines cannot flood the
/// terminal with blank rows either.
const MAX_PROMPT_COMMAND_LINES: usize = 32;
/// The most characters the prompt paints as its tail: the operator sees where
/// the command ends even when the middle is saved.
const MAX_PROMPT_COMMAND_TAIL_CHARS: usize = 512;

/// The command as the operator reads it: sanitised, and — when it does not
/// fit the display budget — the head and tail with the withheld byte count
/// and the private file holding the exact bytes between them. A command that
/// fits is shown whole. When no file can be written, the whole command is
/// shown rather than approve a tail the operator never saw.
fn shown_command(command: &str, approval_dir: Option<&Path>, workspace: Option<&Path>) -> String {
    if !over_display_bound(command) {
        return sanitise_prompt_text(command);
    }
    let head = head_of(command);
    let tail = &command[tail_start(command)..];
    let withheld = command.len() - head.len() - tail.len();
    let saved = approval_dir.and_then(|dir| command_file::save(dir, workspace, command).ok());
    let Some(path) = saved else {
        return sanitise_prompt_text(command);
    };
    format!(
        "{} … [{} bytes not shown; full command saved to {}] … {}",
        sanitise_prompt_text(head),
        withheld,
        sanitise_prompt_text(&path.display().to_string()),
        sanitise_prompt_text(tail),
    )
}

/// Whether the command's head and a distinct tail cannot both fit: the head
/// is cut and the tail starts beyond it, so bytes in between would be hidden.
fn over_display_bound(command: &str) -> bool {
    let head = head_of(command);
    head.len() < command.len() && tail_start(command) > head.len()
}

/// The head the prompt paints: the first [`MAX_PROMPT_COMMAND_CHARS`]
/// characters, cut at the [`MAX_PROMPT_COMMAND_LINES`]th newline when the
/// command has that many lines.
fn head_of(command: &str) -> &str {
    let end = command
        .char_indices()
        .nth(MAX_PROMPT_COMMAND_CHARS)
        .map(|(index, _)| index)
        .unwrap_or(command.len());
    match nth_newline(&command[..end], MAX_PROMPT_COMMAND_LINES) {
        Some(cut) => &command[..cut],
        None => &command[..end],
    }
}

/// The byte index where the command's last
/// [`MAX_PROMPT_COMMAND_TAIL_CHARS`] characters begin, moved forward when
/// those characters hold more than [`MAX_PROMPT_COMMAND_LINES`] line breaks so
/// a command of newlines cannot flood the prompt from the tail either.
fn tail_start(command: &str) -> usize {
    let total = command.chars().count();
    let start = if total <= MAX_PROMPT_COMMAND_TAIL_CHARS {
        0
    } else {
        command
            .char_indices()
            .nth(total - MAX_PROMPT_COMMAND_TAIL_CHARS)
            .map(|(index, _)| index)
            .unwrap_or(command.len())
    };
    let mut cut = start;
    for (seen, (index, _)) in command[start..].match_indices('\n').rev().enumerate() {
        if seen + 1 == MAX_PROMPT_COMMAND_LINES {
            cut = start + index + 1;
            break;
        }
    }
    cut
}

/// The byte index just past the `line`th newline, when the text has that many.
fn nth_newline(text: &str, line: usize) -> Option<usize> {
    text.match_indices('\n')
        .nth(line - 1)
        .map(|(index, _)| index + 1)
}

/// Whether an answered ask still binds the command that will run to the bytes
/// the operator was shown: an over-bound `shell` command's saved file must
/// hold exactly the command. The file is removed either way. `true` where no
/// over-bound command was shown.
pub(super) fn answer_binding_holds(name: &str, arguments: &Value, facts: &ApprovalFacts) -> bool {
    if operation(name) != Some(Operation::Run) {
        return true;
    }
    let Some(command) = arguments.get("command").and_then(Value::as_str) else {
        return true;
    };
    let Some(dir) = facts.approval_dir.as_deref() else {
        return true;
    };
    if !over_display_bound(command) {
        return true;
    }
    let path = command_file::path(dir, command);
    let held = command_file::matches(&path, command);
    command_file::remove(&path);
    held
}

/// `shell`'s fact lines. The route line states which route the call takes: a
/// `sandbox: true` call is confined to the checkout, and a host call runs as
/// the user. The workdir line states the containment the shell enforces.
pub(super) fn facts(
    arguments: &Value,
    facts: &ApprovalFacts,
    session_line: Option<String>,
) -> Option<String> {
    let command = arguments.get("command").and_then(Value::as_str)?;
    if command.is_empty() {
        return None;
    }
    let sandboxed = arguments.get("sandbox") == Some(&Value::Bool(true));
    let mut lines = vec![format!(
        "  command: {}",
        shown_command(
            command,
            facts.approval_dir.as_deref(),
            facts.workspace_root.as_deref()
        )
    )];
    if let Some(workdir) = arguments
        .get("workdir")
        .and_then(Value::as_str)
        .filter(|workdir| !workdir.is_empty())
    {
        lines.push(format!("  workdir: {}", sanitise_prompt_text(workdir)));
    }
    if sandboxed {
        lines.push(
            "  lane: the sandbox — writes reach only the checkout, there is no network, and \
             the environment is empty"
                .to_string(),
        );
    } else {
        lines.push(
            "  lane: this machine — runs as your user, unsandboxed, with your whole \
             filesystem and your network"
                .to_string(),
        );
    }
    if let Some(root) = facts.workspace_root.as_deref() {
        lines.push(format!(
            "  workdir is confined to this session's workspace root: {}",
            root.display()
        ));
    }
    lines.push(format!(
        "  command ≤ {MAX_COMMAND_BYTES} bytes · deadline ≤ {}s · output capped and redacted \
         before it reaches the model",
        MAX_TIMEOUT_MILLIS / 1000
    ));
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    Some(body(
        "shell — runs a command in this session's workspace".to_string(),
        lines,
    ))
}
