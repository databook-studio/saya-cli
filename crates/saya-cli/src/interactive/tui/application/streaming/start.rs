//! Starts the background agent prompt for one turn.

use super::super::super::agent::{self};
use super::super::super::types::App;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use std::sync::Arc;

impl App {
    /// Starts streaming an agent prompt on a background thread. The turn's
    /// decider is built over the session's one approval policy — synced to
    /// the current mode (a mid-session `/approval` takes effect next turn,
    /// carrying every grant the user made) and cloned into `StreamRequest`,
    /// so a grant recorded in this turn is in force for the next.
    pub(crate) fn start_agent(
        &mut self,
        prompt: String,
        state: &SessionState,
        session: &mut SessionRuntime,
    ) {
        let approval = state
            .approval_mode
            .parse()
            .unwrap_or(saya_agent::ApprovalPolicy::Ask);
        session.sync_policy(approval);
        // The live task list starts the turn seeded from the record — the
        // headless loop's seam — so the model sees what it last wrote. The
        // sync-back lands in `drain_stream`, where the turn settles.
        self.session.seed_tasks(state.task_list.clone());
        self.request.stream = Some(agent::start(agent::StreamRequest {
            runtime: Arc::clone(&self.runtime),
            prompt,
            approval,
            policy: session.policy(),
            overrides: state.prompt_overrides(),
            history: state.provider_history(),
            state_db: self.state_db.clone(),
            last_sql: self.last_query.as_ref().map(|lq| lq.sql.clone()),
            session: Arc::clone(&self.session),
            journal: Some(session.journal()),
            agent_mode: state.agent_mode_parsed(),
        }));
        self.request.started = Some(std::time::Instant::now());
    }
}
