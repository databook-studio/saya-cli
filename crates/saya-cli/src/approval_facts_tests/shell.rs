//! The shell prompt's command echo: the schema admits a 64 KiB command, so
//! the prompt paints a bounded head and tail, and names a private file
//! holding the exact bytes the operator is approving. A command that fits is
//! shown whole and writes no file.

use super::*;
use crate::agent::tools::shell_policy::{MAX_COMMAND_BYTES, definition};
use std::path::{Path, PathBuf};

/// A fresh state directory per test, the way a session's is created.
fn temp_state_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-approval-shell-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("the temp state dir is created");
    dir
}

fn facts_with_approval_dir(dir: &Path) -> ApprovalFacts {
    ApprovalFacts {
        approval_dir: Some(dir.to_path_buf()),
        ..session_facts()
    }
}

/// The file path the prompt names, parsed from the marker.
fn saved_path(prompt: &str) -> PathBuf {
    let after = prompt
        .split("full command saved to ")
        .nth(1)
        .unwrap_or_else(|| panic!("the prompt names the saved file: {prompt}"));
    PathBuf::from(
        after
            .split(']')
            .next()
            .expect("the marker closes after the path"),
    )
}

/// A 64 KiB command is admitted by the schema, and the prompt stays bounded:
/// it shows the head and tail, states how many bytes it did not paint, and
/// names a private file holding the exact command.
#[test]
fn a_64_kib_shell_command_renders_bounded() {
    let dir = temp_state_dir("64-kib");
    let facts = facts_with_approval_dir(&dir);
    let command = format!("HEAD{}TAIL", "x".repeat(MAX_COMMAND_BYTES - 8));
    let arguments = serde_json::json!({"command": command});
    let prompt = approval_prompt(&definition(), &arguments, None, &facts, None, None);
    assert!(
        prompt.len() < 8 * 1024,
        "the prompt stays bounded: {} bytes",
        prompt.len()
    );
    assert!(prompt.contains("HEAD"), "the head is shown: {prompt}");
    assert!(prompt.contains("TAIL"), "the tail is shown: {prompt}");
    assert!(
        prompt.contains(&format!("{} bytes not shown", command.len() - 2_048 - 512)),
        "the prompt states the bytes it did not paint: {prompt}"
    );
    let path = saved_path(&prompt);
    let bytes = std::fs::read(&path).expect("the saved command exists");
    assert_eq!(
        bytes,
        command.as_bytes(),
        "the saved file is the exact command"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path)
            .expect("the saved command has metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the saved command is private");
    }
    assert!(
        prompt.contains(&format!("command ≤ {MAX_COMMAND_BYTES} bytes")),
        "the prompt still states the enforced ceiling: {prompt}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A command made of newlines cannot flood the prompt with blank rows either.
#[test]
fn a_newline_heavy_command_renders_bounded() {
    let dir = temp_state_dir("newlines");
    let facts = facts_with_approval_dir(&dir);
    let arguments = serde_json::json!({"command": "\n".repeat(MAX_COMMAND_BYTES)});
    let prompt = approval_prompt(&definition(), &arguments, None, &facts, None, None);
    assert!(
        prompt.lines().count() <= 80,
        "the prompt stays bounded: {} lines",
        prompt.lines().count()
    );
    assert!(
        prompt.contains("bytes not shown"),
        "the prompt says what it did not show: {prompt}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A command under the display bound is shown whole and writes no file: the
/// file exists only to carry bytes the prompt could not paint.
#[test]
fn an_under_bound_shell_command_writes_no_file() {
    let dir = temp_state_dir("under-bound");
    let facts = facts_with_approval_dir(&dir);
    let arguments = serde_json::json!({"command": "echo hi"});
    let prompt = approval_prompt(&definition(), &arguments, None, &facts, None, None);
    assert!(prompt.contains("command: echo hi"), "{prompt}");
    assert!(!prompt.contains("saved to"), "{prompt}");
    assert!(
        crate::approval_facts::saved_commands(&dir).is_empty(),
        "a command the prompt shows whole needs no file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
