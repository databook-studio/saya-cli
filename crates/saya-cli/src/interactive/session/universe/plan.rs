// The session's plan gate: the one place the plan posture's write surface
// opens.
//
// A session under `plan` advertises the read-shaped surface — the posture
// denies an external side effect and a workspace write — plus the gate's own
// `submit_plan`. The model states the plan, the operator answers that one
// call, and an approval opens the write, shell and MCP families for as long
// as the posture stands; leaving `plan` closes the gate again
// (`permissions::mode::switch`).
//
// The gate itself is `agent::tools::coding::plan_gate`'s — the same tool, the
// same bound, the same wording a coding job composes — so no second plan
// implementation exists. What is the session's is the *operator*: [`PlanAsk`]
// bridges the gate to the decider this turn's asks already go through, so the
// plan is put to the one prompt implementation the surface has (the TUI's
// modal, the REPL's terminal prompt), never a second one.

// The module's own imports — `Arc`, `ApprovalDecider`, `PlanGate` and
// `async_trait` — are the ones this included file shares with its siblings
// (`mod.rs`, `shell.rs`, `executors.rs`); only what none of them names is
// imported here.
use crate::agent::tools::consent::Operator;

/// The session's plan gate put to this turn's approval surface.
///
/// The gate's operator is fixed when the gate is composed, and the surface
/// that answers it is the turn's. So the adapter holds a cell the turn binds
/// before the model runs — the same per-turn binding the shell's and the MCP
/// lane's operators ride — and asks through whatever decider is bound there.
#[derive(Default)]
pub(crate) struct PlanAsk {
    /// This turn's approval surface. `None` — a turn that bound none — refuses
    /// every submission: there is no reader, so no plan is approved.
    decider: std::sync::Mutex<Option<Arc<dyn ApprovalDecider>>>,
}

impl PlanAsk {
    /// Binds this turn's approval surface, or `None` for a turn without one.
    pub(crate) fn bind(&self, decider: Option<Arc<dyn ApprovalDecider>>) {
        let mut slot = self
            .decider
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = decider;
    }
}

#[async_trait::async_trait]
impl Operator for PlanAsk {
    /// The subject line the gate builds is dropped, exactly as
    /// `DeciderShellOperator` drops its own: the decider renders the call's
    /// own fact body (`approval_facts::call_facts`), which is where the plan
    /// the operator is answering is stated.
    async fn ask(&self, _subject: &str, arguments: &serde_json::Value) -> bool {
        let decider = self
            .decider
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match decider {
            Some(decider) => decider.approve(&PlanGate::definition(), arguments).await,
            None => false,
        }
    }
}

/// The session's plan gate and its operator, as one pair: the gate holds the
/// operator and the universe holds both, so `bind_plan_ask` reaches the very
/// cell the gate asks through — the two can never be two different cells.
pub(crate) fn plan_gate_pair() -> (Arc<PlanGate>, Arc<PlanAsk>) {
    let ask = Arc::new(PlanAsk::default());
    let operator: Arc<dyn Operator> = ask.clone();
    (
        Arc::new(PlanGate::new(
            Some(operator),
            crate::agent::tools::consent::Caller::Session,
        )),
        ask,
    )
}

impl SessionUniverse {
    /// The session's plan gate: composed once per session, so an operator's
    /// approval — and the close a mode switch performs — survive every turn.
    pub(crate) fn plan_gate(&self) -> Arc<PlanGate> {
        Arc::clone(&self.plan)
    }

    /// Whether the plan gate is open. The one fact the advertisement reads to
    /// decide the write, shell and MCP families under `plan`; the engine reads
    /// the same gate's cell, so the two cannot disagree.
    pub(crate) fn plan_gate_open(&self) -> bool {
        self.plan.is_open()
    }

    /// Binds this turn's approval surface onto the gate. Called once per turn,
    /// before the model runs, with the same decider every other ask of the
    /// turn goes through.
    pub(crate) fn bind_plan_ask(&self, decider: Option<Arc<dyn ApprovalDecider>>) {
        self.plan_ask.bind(decider);
    }
}
