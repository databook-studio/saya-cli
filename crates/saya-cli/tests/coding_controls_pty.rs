#![cfg(not(windows))]
//! Real PTY acceptance source for selected live coding controls.
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(20);
const INTERACTIVE: Duration = Duration::from_secs(15);
include!("coding_controls_pty/session.rs");
include!("coding_controls_pty/provider.rs");
include!("coding_controls_pty/case.rs");
