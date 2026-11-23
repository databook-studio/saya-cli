#![cfg(not(windows))]

//! The vt100 screens `REPORT.md` quotes, from the real binary on a pty.
//!
//! Ignored by default — a capture harness, not an assertion:
//! `cargo test -p saya-cli --test ui7_captures -- --ignored --nocapture`.
//! `SAYA_PTY_BINARY` names the binary to launch, so the same launch can be
//! caught from the commit before the change and from the change itself. Each
//! screen prints its plain rows and the rows' foreground colour runs: a
//! highlight and a meter are paint, and text alone would show neither.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(900);
const DEADLINE: Duration = Duration::from_secs(40);
const READ: Duration = Duration::from_millis(120);
/// A listed id, so the window the meter divides by is known to the launch.
const MODEL: &str = "gpt-4o";
const SENTINEL: &str = "small.";
const ANSWER: &str = "Here are three snippets.\n\n\
```rust\nfn main() {\n    let total = 42;\n    println!(\"{total}\");\n}\n```\n\n\
```python\ndef order_total(items):\n    return len(items) * 2\n```\n\n\
```sql\nSELECT id, name FROM users WHERE id = 1;\n```\n\n\
The three snippets above are small.";

include!("ui7_captures/support/screen.rs");
include!("ui7_captures/support/gateway.rs");
include!("ui7_captures/support/launch.rs");

/// One run at one size: the answer's screen, then `/status`'s.
///
/// The whole five-section panel is taller than 30 rows, so at the review sizes
/// the capture waits for the panel's tail and at the taller sizes it waits for
/// the model section itself — the rows this job fills. Both are printed: the
/// screen a 30-row terminal shows, and the line the panel carries.
fn capture(size @ (cols, rows): (u16, u16), priced: bool) {
    let home = scratch_home(&format!("{cols}x{rows}"));
    let (base_url, stop) = provider(priced);
    let mut cap = launch(&home, &base_url, size);
    cap.wait_for("trust this folder?");
    cap.send("y");
    cap.wait_for("workspace trusted");
    cap.send("show me the snippets\r");
    cap.wait_for(SENTINEL);
    cap.settle();
    cap.print(&format!("ANSWER {cols}x{rows}"));
    cap.send("/status\r");
    cap.wait_for(if rows >= 40 {
        "Context use"
    } else {
        "limits.job.max_cost_usd"
    });
    cap.settle();
    cap.print(&format!("STATUS {cols}x{rows}"));
    let _ = stop.send(());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "the capture harness: run with --ignored --nocapture"]
fn ui7_screens() {
    // An unpriced provider: the meter shows the share and no money.
    capture((100, 30), false);
    capture((60, 24), false);
    // A provider that prices the request: the meter adds the session's spend.
    capture((100, 60), true);
    capture((60, 50), true);
}
