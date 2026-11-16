//! `saya sessions list|delete` through the real binary.
//!
//! Saved-session management must be explicit and never automatic: listing
//! mutates nothing, delete removes exactly the named saved session (its
//! record, state directory, and transient copies) and says plainly what it
//! did not delete, a live session is refused by pid, and no path ever
//! prompts or reads stdin. Every refusal exits 2, the documented usage
//! class.

#[path = "session_lifecycle_cli/deletion.rs"]
mod deletion;
#[path = "session_lifecycle_cli/listing.rs"]
mod listing;
#[path = "session_lifecycle_cli/stdin.rs"]
mod stdin;
#[path = "session_lifecycle_cli/support.rs"]
mod support;
