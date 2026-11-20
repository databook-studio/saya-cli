//! A helper's write-shaped calls under a job with an operator attached: the
//! same ask the root uses, under a line that names the helper.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ApprovalPolicy, SessionGrants, ToolDefinition};
use saya_harness::coding::policy::ChildAuthorityView;
use saya_types::{AgentId, JobId, ManagedCheckoutId};
use serde_json::{Value, json};

use super::decider::ExactWriteChildApproval;
use super::tests::{Sandbox, checkout, implementer_view};
use super::{CliChildFactory, child_toolset};
use crate::agent::tools::WriteAsk;
use crate::agent::tools::consent::{Answer, Caller, Operator};
use crate::agent::tools::shell_policy::ModeCell;

const AGENT: &str = "helper-2";
const JOB: &str = "job-writes";
const HELPER_LINE: &str = "Helper helper-2 wants to change a workspace file for job job-writes";

/// An operator that answers with the choice the test states and records the
/// subject lines it is shown.
struct Recording {
    answer: Answer,
    grants: SessionGrants,
    subjects: Mutex<Vec<String>>,
}

impl Recording {
    fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            grants: SessionGrants::default(),
            subjects: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.subjects.lock().unwrap().len()
    }

    fn subjects(&self) -> Vec<String> {
        self.subjects.lock().unwrap().clone()
    }
}

#[async_trait]
impl Operator for Recording {
    fn grants(&self) -> Option<SessionGrants> {
        Some(self.grants.clone())
    }

    async fn ask(&self, subject: &str, _arguments: &Value) -> bool {
        self.subjects.lock().unwrap().push(subject.to_owned());
        !matches!(self.answer, Answer::Deny)
    }

    async fn answer(&self, subject: &str, _arguments: &Value) -> Answer {
        self.subjects.lock().unwrap().push(subject.to_owned());
        self.answer
    }
}

/// The helper's write ask over one stated mode cell.
fn ask(mode: ApprovalPolicy, operator: Arc<Recording>) -> Arc<WriteAsk> {
    Arc::new(WriteAsk::new(
        ModeCell::new(mode),
        operator,
        &Caller::Helper {
            agent_id: AgentId::parse(AGENT).unwrap(),
            job_id: JobId::parse(JOB).unwrap(),
        },
    ))
}

/// The helper's `edit` and the authority it is admitted by, as its own view
/// advertises it.
fn edit(sandbox: &Sandbox, label: &str) -> (ToolDefinition, ChildAuthorityView, ManagedCheckoutId) {
    let checkout = checkout(label);
    let view = implementer_view(&checkout);
    let pair = sandbox.pair(&checkout);
    let definition = child_toolset(&view, &pair)
        .unwrap()
        .0
        .into_iter()
        .find(|definition| definition.name == "edit")
        .expect("the helper advertises edit");
    (definition, view, checkout)
}

/// The helper's exact-write decider, carrying `operator`'s ask.
fn approval(
    view: &ChildAuthorityView,
    checkout: &ManagedCheckoutId,
    operator: Arc<Recording>,
    mode: ApprovalPolicy,
) -> ExactWriteChildApproval {
    ExactWriteChildApproval::new(view, checkout.clone()).with_ask(Some(ask(mode, operator)))
}

/// Under `ask`, a helper's edit reaches the operator once, under a line that
/// names the helper, and the answer decides.
#[tokio::test]
async fn ask_puts_a_helpers_edit_to_the_operator_under_the_helper_line() {
    let sandbox = Sandbox::new("helper-write-ask");
    let (edit, view, checkout) = edit(&sandbox, "helper-write-ask");
    let call = json!({"path": "src/lib.rs", "old_text": "a", "new_text": "b"});
    let operator = Recording::new(Answer::Once);
    let admitted = approval(&view, &checkout, operator.clone(), ApprovalPolicy::Ask);

    assert!(
        admitted.approve(&edit, &call).await,
        "an approving operator admits the edit"
    );
    assert_eq!(operator.calls(), 1);
    assert_eq!(operator.subjects(), vec![HELPER_LINE.to_owned()]);

    let refusing = Recording::new(Answer::Deny);
    let denied = approval(&view, &checkout, refusing.clone(), ApprovalPolicy::Ask);
    assert!(!denied.approve(&edit, &call).await);
    assert_eq!(refusing.calls(), 1);
}

/// A "yes for this run" on a helper's edit covers the job's next write: the
/// grant lives on the job's operator, not on one caller.
#[tokio::test]
async fn a_helpers_session_answer_covers_the_next_write() {
    let sandbox = Sandbox::new("helper-write-session");
    let (edit, view, checkout) = edit(&sandbox, "helper-write-session");
    let call = json!({"path": "src/lib.rs", "old_text": "a", "new_text": "b"});
    let operator = Recording::new(Answer::Session);
    let approval = approval(&view, &checkout, operator.clone(), ApprovalPolicy::Ask);

    assert!(approval.approve(&edit, &call).await);
    assert!(operator.grants.is_granted("workspace-write"));
    assert!(approval.approve(&edit, &call).await);
    assert_eq!(operator.calls(), 1, "the second edit is not asked");
}

/// The capability decides first here too: a call the child authority refuses
/// never reaches the operator, and no mode can admit it.
#[tokio::test]
async fn a_helpers_write_outside_the_checkout_never_reaches_the_operator() {
    let sandbox = Sandbox::new("helper-write-escape");
    let (edit, view, checkout) = edit(&sandbox, "helper-write-escape");
    let escape = json!({"path": "../escape.txt", "old_text": "a", "new_text": "b"});

    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::Bypass,
    ] {
        let operator = Recording::new(Answer::Once);
        let approval = approval(&view, &checkout, operator.clone(), mode);

        assert!(
            !approval.approve(&edit, &escape).await,
            "{mode:?} keeps the capability's refusal"
        );
        assert_eq!(operator.calls(), 0, "{mode:?} asks nobody");
    }
}

/// The factory the job composes a helper with carries the same ask, so the
/// decision the helper's loop reads is the one the job stated.
#[tokio::test]
async fn the_factory_carries_the_jobs_write_ask_to_a_helpers_decider() {
    let sandbox = Sandbox::new("helper-write-factory");
    let checkout = checkout("helper-write-factory");
    let view = implementer_view(&checkout);
    let operator = Recording::new(Answer::Once);
    let factory = CliChildFactory::new(
        sandbox.pair(&checkout),
        super::tests_factory::test_ai(),
        Arc::new(saya_config::MapSecretResolver::new([])),
    )
    .with_write_ask(Some(ask(ApprovalPolicy::Ask, operator.clone())));
    let (definitions, _tools, approval) = factory.build_toolset(&view).unwrap();
    let edit = definitions
        .into_iter()
        .find(|definition| definition.name == "edit")
        .expect("the helper advertises edit");

    assert!(
        approval
            .approve(
                &edit,
                &json!({"path": "src/lib.rs", "old_text": "a", "new_text": "b"})
            )
            .await
    );
    assert_eq!(
        operator.calls(),
        1,
        "the composed decider asks the operator"
    );
    assert_eq!(operator.subjects(), vec![HELPER_LINE.to_owned()]);
}
