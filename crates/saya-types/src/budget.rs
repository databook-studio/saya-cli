//! The agent's message-byte budget, shared by the two crates that must agree on it.
//!
//! [`MAX_MESSAGE_BYTES`] is the ceiling `saya-agent` enforces on the system +
//! user + history messages it sends to a provider. The prompt-recall path bounds
//! the context block against what is left of this after the system prompt and the
//! user's own question, so memory never crowds out the question it is meant to
//! help. `[memory] max_context_bytes` is clamped below a ceiling derived from this
//! same constant, so a user who turns the setting up cannot exhaust the budget
//! the question needs.
//!
//! Owned here — not in `saya-agent` or `saya-config` — because both mid-layer
//! crates read it and the architecture forbids a sideways `saya-config →
//! saya-agent` dependency. `saya-agent` re-exports it as [`super::MAX_HISTORY_BYTES`]
//! (the name its history module and tests already use); `saya-config` derives the
//! `max_context_bytes` ceiling from it. One number, two consumers, no drift.

/// The whole-message byte budget the agent enforces: system + user + history
/// must fit under this before the request is sent. The prompt-recall path
/// reserves the system prompt and the user's question out of this first, then
/// gives context what remains.
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;
