//! O04 `/reload` on the real binary: the piped REPL, `--turn-file`, and the
//! privacy regression. Piped stdin/stdout keeps this the headless path (no
//! trust prompt, no editor), so the loop reads lines and prints results we
//! can wait for — a reader thread per stream plus a wait-for-substring
//! helper, as `tests/mvp/session.rs` does. The config home is isolated via
//! `SAYA_CONFIG_HOME`, and every inherited `SAYA_*` variable is removed so
//! the reload re-reads exactly the files this test writes.
//!
//! No provider and no database: slash commands need neither.

#[path = "reload_repl/arc.rs"]
mod arc;
#[path = "reload_repl/privacy.rs"]
mod privacy;
#[path = "reload_repl/support.rs"]
mod support;
#[path = "reload_repl/turn_file.rs"]
mod turn_file;
