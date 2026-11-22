//! The one trust decision, as both surfaces state it.
//!
//! A launch that bound no workspace asks once whether to bind the launch folder
//! (or a typed directory) or to continue unbound. The answer is this enum: the
//! new surface's `y`/`n` produce the same two resolutions the old modal's `t`
//! and `c` produced, and the store behind either is the same call
//! (`SessionRuntime::bind_trusted`, exactly like an explicit `--workspace`).
//!
//! The old modal's key grammar — the draft typed after `w`, the typed-directory
//! answer, the inline error — went with it in X1: the new surface reads its own
//! key pair (`y` binds the launch folder, `n` continues), so only the answer is
//! shared.

/// The modal's answer once a key resolves to one: bind this folder, bind the
/// typed directory, or close unbound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrustResolution {
    /// Bind exactly the launch cwd, canonicalised — no parent, no walk.
    TrustCwd,
    /// Bind nothing: today's unbound shape.
    ContinueUnbound,
}
