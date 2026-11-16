//! The root shell's test fixtures: a scratch checkout and the counting and
//! approving fake operators the battery and its host-lane sibling both build
//! on.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_harness::runner::HostPolicy;
use saya_types::JobId;
use serde_json::{Value, json};

use super::super::root_shell::{self, RootShell};
use crate::agent::tools::consent::{Caller, Operator, Prompt};
use crate::agent::tools::shell_policy::HOST_APPROVAL_UNAVAILABLE;
use crate::agent::tools::shell_policy::prompt as shell_prompt;

/// The refusal a host call earns under `ask` with no operator to answer it:
/// the policy's own text, never a copy of it, so the two cannot drift.
pub(crate) const NO_OPERATOR_REFUSAL: &str = HOST_APPROVAL_UNAVAILABLE;
/// The refusal a host call earns under `read-only`.
pub(crate) const READ_ONLY_REFUSAL: &str = "this session is read-only: the shell runs only inside \
     the sandbox, where nothing in the checkout is writable";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// A per-test scratch checkout: created, used, removed on drop.
pub(crate) struct TempCheckout(PathBuf);

impl TempCheckout {
    pub(crate) fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "saya-root-shell-{tag}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join(".git")).expect("the checkout root must be creatable");
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempCheckout {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// An operator that answers a fixed verdict and counts every call.
pub(crate) struct RecordingOperator {
    approve: bool,
    calls: Mutex<usize>,
}

impl RecordingOperator {
    pub(crate) fn new(approve: bool) -> Arc<Self> {
        Arc::new(Self {
            approve,
            calls: Mutex::new(0),
        })
    }

    pub(crate) fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl Operator for RecordingOperator {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        *self.calls.lock().unwrap() += 1;
        self.approve
    }
}

/// An operator that approves every call.
pub(crate) struct ApproveAll;

#[async_trait]
impl Operator for ApproveAll {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        true
    }
}

/// The prompt these fixtures build their shells under: the root's own subject
/// line, naming one job. What the line says is [`Caller`]'s business;
/// these tests only ever count the operator's calls.
pub(crate) fn prompt(operator: Option<Arc<dyn Operator>>) -> Option<Prompt> {
    operator.map(|operator| {
        shell_prompt(
            operator,
            Caller::Root {
                job_id: JobId::parse("job").unwrap(),
            },
        )
    })
}

/// A root shell over `checkout` with a proven sandbox, or `None` where the
/// sandbox does not prove on this host.
pub(crate) fn shell_over(
    checkout: &Path,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
) -> Option<RootShell> {
    root_shell::build_with_policy(checkout, policy, prompt(operator), CancellationToken::new())
}

/// A root shell over the host lane alone, as a host with no proven sandbox
/// builds one.
pub(crate) fn host_shell_over(
    checkout: &Path,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
    host: HostPolicy,
) -> Option<RootShell> {
    host_shell_over_with_spool(checkout, policy, operator, host, None)
}

/// As [`host_shell_over`], with the shell's full captured output saved under
/// `spool` (the job's private state directory).
pub(crate) fn host_shell_over_with_spool(
    checkout: &Path,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
    host: HostPolicy,
    spool: Option<&Path>,
) -> Option<RootShell> {
    root_shell::build_host_only_with_spool(
        checkout,
        policy,
        prompt(operator),
        host,
        CancellationToken::new(),
        spool,
    )
}

/// A call that names no lane: `sandbox` is omitted, so the schema default —
/// the host lane — applies.
pub(crate) fn call(command: &str) -> Value {
    json!({ "command": command, "description": "a probe" })
}

/// A call that names the host lane explicitly.
pub(crate) fn host_call(command: &str) -> Value {
    json!({ "command": command, "description": "a probe", "sandbox": false })
}

/// A call that names the sandbox lane explicitly.
pub(crate) fn sandbox_call(command: &str) -> Value {
    json!({ "command": command, "description": "a probe", "sandbox": true })
}
