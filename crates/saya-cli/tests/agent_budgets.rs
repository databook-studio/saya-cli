//! Proves the env-driven budget ceilings at the real binary boundary.
//!
//! The ceilings the runtime reads from the process environment
//! (`runtime.rs` composes them onto `AgentLimits`) are proven by spawning the
//! real headless `saya ask` with the ceiling variables planted **on the child
//! `Command` only** — never in this test process, whose parallel tests run
//! the same runtime and must never observe a planted ceiling. The stop point
//! is observed from what the scripted mock provider received (request count,
//! whether each request carried tools) and the child's output/exit.
//!
//! The mock harness is a trimmed copy of `tests/correctness/common.rs`
//! (tests in different binaries cannot share modules), with one adaptation:
//! the thread serves the script, then an unbounded text fallback, until a
//! quiet period with no new connection passes — so a run stopped early by a
//! ceiling still exits and `join` cannot hang.

use std::{
    fs,
    io::{self, Read},
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// The ceiling variables the runtime composes onto `AgentLimits` (the names
/// `budgets_from_env` reads). Scrubbed from every child, then planted per
/// scenario, so no scenario inherits an ambient value.
const BUDGET_VARS: [&str; 3] = [
    "SAYA_AGENT_MAX_TURNS",
    "SAYA_AGENT_MAX_TOOL_CALLS",
    "SAYA_AGENT_MAX_CONTINUATIONS",
];

/// The ask can always answer: after the script the mock serves plain text,
/// so the process exits on its own. Request *shape* is the oracle, not the
/// answer — a swapped or dropped ceiling changes how many scripted rounds
/// the loop consumes, and whether the last request stripped its tools.
const FALLBACK_TEXT: &str = "fallback answer from the script-exhausted mock";

/// The text answer the ceiling scenarios share: it sits where the ceiling
/// salvage call lands, so the planted regime and the dropped-ceiling regime
/// terminate on the SAME words — what differs is the requests' tools
/// signature, never the prose.
const CEILING_TEXT: &str = "the orders count is 560";

/// How long the mock waits for one more connection before returning: long
/// enough that a healthy run's next round arrives, short enough that a
/// budget-stopped run's `join` returns instead of hanging. Reset on every
/// accepted connection.
const QUIET_AFTER_LAST: Duration = Duration::from_secs(8);

/// An isolated, pid-suffixed scratch root for one scenario.
fn test_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-agent-budgets-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

include!("agent_budgets/provider.rs");
include!("agent_budgets/fixture.rs");
include!("agent_budgets/scenario.rs");
include!("agent_budgets/ceilings.rs");
