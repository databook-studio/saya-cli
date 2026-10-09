//! The M5-3 sandbox module's integration tests — the fail-closed contract,
//! measured against the real `sandbox-exec` on this host where one exists.
//!
//! Six guarantees, one per red test the milestone demanded:
//! 1. Probe failure ⇒ the runner is not registered ⇒ a plan requesting the
//!    runner scope is refused. Asserted on the tool list and the typed plan
//!    refusal, never on a flag.
//! 2. A read outside `fs_roots` is denied, with `Operation not permitted` in
//!    stderr.
//! 3. A write outside `fs_roots` is denied; a write inside succeeds and the
//!    file exists afterwards.
//! 4. Egress to a port not in `net_allow` is denied with `Operation not
//!    permitted`; an allowed port connects — against a listener that
//!    actually calls `accept()` (the spike's starved-listener lesson).
//! 5. `#[cfg(windows)]`: profile construction refuses the unsupported root,
//!    so the runner is not registered.
//! 6. The generated profile is rejected by `sandbox-exec` if a placeholder
//!    was left unsubstituted — and because a *quoted* leftover is silently
//!    accepted (measured on this host: exit 0), the generator itself refuses
//!    leftovers.

use std::{fs, path::PathBuf};

#[cfg(unix)]
use std::{collections::VecDeque, sync::Mutex};
#[cfg(target_os = "macos")]
use std::{
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[cfg(unix)]
use async_trait::async_trait;
#[cfg(unix)]
use saya_agent::{ChatMessage, ChatProvider, ChatRequest, ChatResponse, ProviderError};
#[cfg(unix)]
use saya_harness::engine::{PlanDriver, PlanError, PlanRejection, PlanRequest};
#[cfg(unix)]
use saya_harness::runner::sandbox::SandboxProvision;
use saya_harness::runner::sandbox::{RunSandbox, SandboxError};
#[cfg(unix)]
use saya_types::{Budgets, Capabilities, RunnerScope};

#[path = "sandbox/fixtures.rs"]
mod fixtures;
use fixtures::*;

#[path = "sandbox/failed_probe.rs"]
mod failed_probe;
// ---------------------------------------------------------------------------
// macOS-only enforcement tests: these run real sandbox-exec children under
// the real generated profile.
// ---------------------------------------------------------------------------

#[path = "sandbox/construction.rs"]
mod construction;
#[path = "sandbox/macos_filesystem.rs"]
mod macos_filesystem;
#[path = "sandbox/macos_network.rs"]
mod macos_network;
#[path = "sandbox/probe_report.rs"]
mod probe_report;
#[path = "sandbox/sandbox_profile.rs"]
mod sandbox_profile;
