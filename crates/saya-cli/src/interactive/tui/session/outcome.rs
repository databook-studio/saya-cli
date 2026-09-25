//! What the TUI's startup trust modal decided.

/// What the TUI's startup trust modal decided: the trusted directory when
/// the modal bound one, or `Unasked` on every other path — modal dismissed
/// unbound, or never opened. `session_loop` pins the record and refreshes
/// the header facts from the rebound session behind an `Answered`.
pub(crate) enum TrustOutcome {
    Answered(std::path::PathBuf),
    Unasked,
}

impl TrustOutcome {
    /// The process exit code: the TUI always exits cleanly here; the trust
    /// answer rides the session, never the exit status.
    pub(crate) fn exit_code(self) -> i32 {
        0
    }
}
