//! The fixture every helper-shell case builds on: the recording and holding
//! operators, the helper's own shell, and the two call shapes.

use std::fs;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_harness::runner::HostPolicy;
use saya_types::{AgentId, JobId};
use serde_json::{Value, json};

use super::super::tests::Sandbox;
use crate::agent::tools::consent::{Caller, Operator};
use crate::agent::tools::shell_policy::{self, Background, ShellPlacement, ShellTool};

/// The helper these tests compose: one named helper of one named job, so the
/// subject line can be asserted exactly.
pub(super) const HELPER: &str = "helper-2";
pub(super) const JOB: &str = "job-l5";
pub(super) const HELPER_LINE: &str = "Helper helper-2 wants to run a shell command for job job-l5";

/// An operator that records every subject it is asked under, counts the calls,
/// and records the most asks it ever had in flight at once.
pub(super) struct RecordingOperator {
    approve: bool,
    subjects: Mutex<Vec<String>>,
    concurrent: AtomicUsize,
    peak: AtomicUsize,
    /// How long each ask holds the operator, so two asks overlap if nothing
    /// serialises them.
    hold: Duration,
}

impl RecordingOperator {
    pub(super) fn new(approve: bool) -> Arc<Self> {
        Arc::new(Self {
            approve,
            subjects: Mutex::new(Vec::new()),
            concurrent: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            hold: Duration::from_millis(20),
        })
    }

    pub(super) fn calls(&self) -> usize {
        self.subjects.lock().unwrap().len()
    }

    pub(super) fn subjects(&self) -> Vec<String> {
        self.subjects.lock().unwrap().clone()
    }

    pub(super) fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Operator for RecordingOperator {
    async fn ask(&self, subject: &str, _arguments: &Value) -> bool {
        self.subjects.lock().unwrap().push(subject.to_owned());
        let now = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.hold).await;
        self.concurrent.fetch_sub(1, Ordering::SeqCst);
        self.approve
    }
}

/// An operator that holds its answer until it is released: it is how a test
/// keeps one ask pending while another waits for the gate.
pub(super) struct HoldingOperator {
    entered: AtomicBool,
    calls: AtomicUsize,
    release: tokio::sync::Semaphore,
}

impl HoldingOperator {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            release: tokio::sync::Semaphore::new(0),
        })
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(super) fn entered(&self) -> bool {
        self.entered.load(Ordering::SeqCst)
    }

    pub(super) fn release(&self) {
        self.release.add_permits(1);
    }
}

#[async_trait]
impl Operator for HoldingOperator {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.store(true, Ordering::SeqCst);
        let _ = self.release.acquire().await;
        true
    }
}

/// The helper's shell, built exactly as `compose_child` builds it: over the
/// sandbox's working checkout, with this helper's own scratch tree, under the
/// subject line that names it. `None` where this host proves no sandbox.
/// `tests_loop` shares it for the loop-boundary case.
pub(in crate::agent::tools::child) fn helper_shell(
    sandbox: &Sandbox,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
) -> Option<Arc<ShellTool>> {
    let root = sandbox.ws_root();
    let scratch = root.join(".saya").join(format!("helper-{HELPER}"));
    let home = scratch.join("home");
    let tmpdir = scratch.join("tmp");
    fs::create_dir_all(&home).expect("the helper scratch home must create");
    fs::create_dir_all(&tmpdir).expect("the helper scratch tmp must create");
    let prompt = operator.map(|operator| {
        shell_policy::prompt(
            operator,
            Caller::Helper {
                agent_id: AgentId::parse(HELPER).unwrap(),
                job_id: JobId::parse(JOB).unwrap(),
            },
        )
    });
    shell_policy::compose(
        ShellPlacement::helper(&root, (&home, &tmpdir)),
        policy,
        prompt,
        HostPolicy::default(),
        CancellationToken::new(),
        // A helper's shell owns its background runs; they end with the helper.
        Background::Owned,
        saya_config::CommandRules::default(),
    )
    .map(Arc::new)
}

/// A call that names no lane: `sandbox` is omitted, so the schema default —
/// the host lane — applies.
pub(super) fn call(command: &str) -> Value {
    json!({ "command": command, "description": "a probe" })
}

/// A call that names the host lane explicitly.
pub(super) fn host_call(command: &str) -> Value {
    json!({ "command": command, "description": "a probe", "sandbox": false })
}
