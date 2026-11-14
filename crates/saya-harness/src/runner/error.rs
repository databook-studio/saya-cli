//! The confined child's typed error surface. Every variant names what the
//! caller needs to correct — the credential seam, the spawn itself, the
//! shell's path or workdir — never a generic failure. The credential seam
//! (`env`), the spawn, and the workspace shell all raise it, so a call's
//! outcome is always one of these variants.

use std::io;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunnerError {
    #[error("shell refused: credential `{credential}` is invalid — {reason}")]
    CredentialInvalid {
        credential: String,
        reason: &'static str,
    },

    #[error("shell failed: credential `{credential}` could not be resolved: {detail}")]
    CredentialUnresolved { credential: String, detail: String },

    #[error("shell failed: the child process could not be started: {source}")]
    Spawn { source: io::Error },

    #[error("shell failed: the child process's exit could not be observed: {source}")]
    WaitFailed { source: io::Error },

    /// A workspace shell call was refused before anything could be spawned:
    /// the command's shape or size, the timeout's bounds, a workdir that
    /// does not resolve inside the workspace root, or a workspace root that
    /// does not resolve at all.
    #[error("workspace shell refused: {detail}")]
    ShellCommandRefused { detail: String },

    /// A workspace shell call asked for the sandbox on a shell that has
    /// none. The call is refused rather than run on the host behind the
    /// caller's back: "no sandbox here" is a fact the caller must act on,
    /// never one the tool may quietly reinterpret.
    #[error(
        "workspace shell refused: this shell has no proven sandbox and the call asked for one \
         (ask for a host run, or compose a sandboxed shell)"
    )]
    ShellSandboxUnavailable,

    /// A workspace shell call asked for a host run on a shell whose caller
    /// did not allow one. Refused rather than confined behind the caller's
    /// back: the two paths are the caller's decision, never the model's.
    #[error(
        "workspace shell refused: this shell does not allow a host run and the call asked for \
         one (run it in the sandbox, or compose a shell that allows the host)"
    )]
    ShellHostNotAllowed,

    /// A [`RunnerSpawn`](crate::runner::sandbox::RunnerSpawn) was offered as a
    /// workspace shell confinement and is not one: the shell runs with no
    /// egress and inside exactly the workspace, and a proven spawn that
    /// disagrees is refused rather than narrowed.
    #[error("workspace shell refused: the proven sandbox is not a shell confinement — {detail}")]
    ShellNotConfined { detail: &'static str },

    /// A background start was refused because this shell already holds the
    /// most runs it may: the cap is a refusal, never a queue, and the caller
    /// is told the number it must come back under.
    #[error(
        "shell refused: this shell already holds {limit} background runs; kill one with shell_kill first"
    )]
    BackgroundLimitReached { limit: usize },

    /// A background call named a run this shell does not hold: an id from
    /// another shell, or one whose record was already read and released.
    #[error("shell refused: this shell holds no background run with id {id}")]
    BackgroundUnknown { id: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell path hands these variants' own text to the model as a tool
    /// error. No text the model reads may name a tool that does not exist.
    #[test]
    fn no_variant_names_the_retired_run_program_tool() {
        let rendered = [
            RunnerError::CredentialInvalid {
                credential: "cred".into(),
                reason: "malformed",
            },
            RunnerError::CredentialUnresolved {
                credential: "cred".into(),
                detail: "unset".into(),
            },
            RunnerError::Spawn {
                source: io::Error::other("spawn"),
            },
            RunnerError::WaitFailed {
                source: io::Error::other("wait"),
            },
            RunnerError::ShellCommandRefused {
                detail: "too long".into(),
            },
            RunnerError::ShellSandboxUnavailable,
            RunnerError::ShellHostNotAllowed,
            RunnerError::ShellNotConfined { detail: "not one" },
            RunnerError::BackgroundLimitReached { limit: 4 },
            RunnerError::BackgroundUnknown { id: 1 },
        ]
        .map(|error| error.to_string());
        for text in rendered {
            assert!(
                !text.contains("run_program"),
                "the shell lane must not name a retired tool: {text}"
            );
            assert!(
                text.contains("shell"),
                "the text must name the tool that refused: {text}"
            );
        }
    }
}
