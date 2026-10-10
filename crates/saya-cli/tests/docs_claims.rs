//! Q04: the release documentation is a set of executable claims. This suite
//! pins only what was chosen to pin, against the real binary and the real
//! tree:
//!
//! - every slash command the 0.4.2 docs sections document appears in the
//!   binary's `/help` output (a real `--turn-file` run, isolated roots);
//! - the instruction caps are asserted through `/context`'s real output,
//!   with the expected numbers built from the config constants;
//! - the 1 KiB preference text bound is asserted through behaviour (the
//!   64-item cap is already covered by `saya-types/tests/project_preference.rs`);
//!
//! No model and no TTY: every run is a slash command or a preferences
//! subcommand, and the provider env points at a dead address.

#[path = "docs_claims/context_limits.rs"]
mod context_limits;
#[path = "docs_claims/help.rs"]
mod help;
#[path = "docs_claims/preferences.rs"]
mod preferences;
#[path = "docs_claims/support.rs"]
mod support;
