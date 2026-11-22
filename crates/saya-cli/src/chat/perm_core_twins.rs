//! The permission pins the old turn body carried, re-homed on the shapes the
//! live catalogue advertises.
//!
//! Every test under this module is the twin of a test that still builds its
//! fixture from `interactive/session_definitions*` — the old body's own
//! advertisement, which 0.5.1 deletes. A twin keeps the old test's *outcome*
//! and changes only what the port rule allows: the tool name (`run_command` →
//! `shell`, `http_fetch` → `web_fetch`, the job root's catalogue), the decider
//! (`TerminalApproval` → the live shell policy and `WriteAsk`, both over
//! `ChatFront::ask`), and the event shape. No refusal is softened, no grant
//! widened, and no bound moved: `REPORT.md` carries the row-by-row accounting.
//!
//! The shapes are the live ones — `agent::tools::shell_policy::definition`,
//! `web::fetch_definition` and the run surface's file tools — so the tests fail
//! if the catalogue the chat session actually advertises stops carrying them.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, ApprovalPolicy, NoopEventSink, SessionGrants, ToolDefinition};
use saya_config::CommandRules;

use super::{ChatEvent, ChatFront, Question, SessionOperator};
use crate::agent::tools::consent::{Answer, Caller, Operator, Prompt};
use crate::agent::tools::shell_policy::ShellPolicy;
use crate::interactive::session::permissions::deny::{SessionDeny, shell_rules};

/// The job root's `shell`: the one command tool every live surface advertises.
pub(super) fn core_shell() -> ToolDefinition {
    crate::agent::tools::shell_policy::definition()
}

/// The job root's `web_fetch`.
pub(super) fn core_web_fetch() -> ToolDefinition {
    crate::agent::tools::web::fetch_definition()
}

/// The run surface's `write` or `edit`, over the workspace-write permit: the
/// definition the job root composes and the chat's file lane derives from.
pub(super) fn core_file_tool(name: &str) -> ToolDefinition {
    crate::agent::tools::DatabaseTools::definitions(false, false, false, true)
        .into_iter()
        .find(|tool| tool.name == name)
        .unwrap_or_else(|| panic!("{name} is a run-surface file tool"))
}

/// The rule set a launch stating `--deny <programs>` hands the live shell: the
/// deny list converted into the trusted rules the shell policy decides first.
pub(super) fn deny_rules(programs: &[&str]) -> CommandRules {
    let deny = SessionDeny::from_names(programs.iter().map(|program| (*program).to_owned()))
        .expect("a bare name builds");
    shell_rules(&CommandRules::default(), &deny).expect("the deny list converts")
}

/// The front a session's questions are put to: it answers every one with the
/// same answer and keeps what it was asked, so a test states both what came
/// back and whether a person was reached at all.
pub(super) struct Scripted {
    answer: Answer,
    seen: AtomicUsize,
    questions: Mutex<Vec<Question>>,
}

impl Scripted {
    pub(super) fn answering(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            seen: AtomicUsize::new(0),
            questions: Mutex::new(Vec::new()),
        })
    }

    /// The questions this front has been put, in order.
    pub(super) fn questions(&self) -> Vec<Question> {
        self.locked().clone()
    }

    /// How many questions this front has been put.
    pub(super) fn asked(&self) -> usize {
        self.seen.load(Ordering::Acquire)
    }

    fn locked(&self) -> MutexGuard<'_, Vec<Question>> {
        self.questions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl ChatFront for Scripted {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, question: &Question) -> Answer {
        self.seen.fetch_add(1, Ordering::AcqRel);
        self.locked().push(question.clone());
        self.answer
    }

    /// A person is present: a question put here is one somebody answers.
    fn can_obtain_approval(&self) -> bool {
        true
    }

    fn emit(&self, _event: ChatEvent) {}
}

/// The run's operator over that front: the same `SessionOperator` the chat
/// session composes, so a yes records into the run's own store.
pub(super) fn operator(front: &Arc<Scripted>) -> Arc<SessionOperator> {
    let operator = Arc::new(SessionOperator::new(front.clone()));
    operator.set_turn(saya_agent::CancellationToken::new());
    operator
}

/// The run's store, the one a session-wide yes lands in.
pub(super) fn grants(operator: &Arc<SessionOperator>) -> SessionGrants {
    Operator::grants(operator.as_ref()).expect("the run's operator keeps a store")
}

/// The live shell policy: the job's mode, the run's operator under the shell's
/// own line, the rules the launch bound, and a sandbox lane present so a mode
/// that must not reach the host has somewhere to serve the call.
pub(super) fn live_shell(
    mode: ApprovalPolicy,
    operator: &Arc<SessionOperator>,
    rules: CommandRules,
) -> ShellPolicy {
    let operator: Arc<dyn Operator> = operator.clone();
    let prompt: Prompt = crate::agent::tools::shell_policy::prompt(operator, Caller::Session);
    ShellPolicy::new(mode, Some(prompt), true, rules, None)
}

#[path = "perm_core_twins/catalog.rs"]
mod catalog;
#[path = "perm_core_twins/deny.rs"]
mod deny;
#[path = "perm_core_twins/facts.rs"]
mod facts;
#[path = "perm_core_twins/grants.rs"]
mod grants;
