#![cfg(not(windows))]

//! Real-terminal proof for the non-TUI approval input owner.
//!
//! The mock's first received request is the source-start boundary. It returns
//! one SQL tool call, which reaches `TerminalApproval`; the configured
//! question deadline must then cancel that pending raw-mode read and let the
//! child settle without a second provider request.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, Once, mpsc},
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(15);

include!("terminal_approval_pty/cases/approval.rs");
include!("terminal_approval_pty/support/pending.rs");
include!("terminal_approval_pty/support/keys.rs");
include!("terminal_approval_pty/support/mock_provider.rs");
include!("terminal_approval_pty/support/pty_guards.rs");
