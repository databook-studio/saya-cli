use super::child_controls::control_error;
use super::{CodingError, NativeCodingJob, delegate::CodingTask};
use saya_harness::coding::{
    child::ChildControl,
    coordinator::{
        ChildOutcome, ScheduleError, ScheduleLimits, ScheduleReport, ScheduleSession,
        ScheduledChild,
    },
};
use saya_types::AgentId;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

/// One live schedule session: the job's side of it while helpers are in flight.
/// `session` admits children and closes the session when dropped; `handle` runs
/// `SessionRun::run` and drains its progress; `controls` are the operator
/// handles of the children it holds; `in_flight` counts the helpers whose own
/// waiter a `task_run` has not consumed yet. The session lives only while that
/// count is above zero — between turns `active` is `None` exactly as before.
///
/// Both `session` and `handle` are taken by the settle: dropping the submit side
/// closes the session so the run drains, and the handle is awaited. The controls
/// stay in `active` while that happens, so an operator can still cancel a child
/// that is still running — the same window the one-shot join left open.
pub(super) struct LiveSession {
    session: Option<ScheduleSession>,
    handle: Option<JoinHandle<Result<ScheduleReport, CodingError>>>,
    controls: BTreeMap<AgentId, ChildControl>,
    in_flight: usize,
}

/// A completion waiter whose sender is already gone: awaiting it resolves to
/// `Err` at once, the "this helper never started" signal.
fn never_started() -> oneshot::Receiver<ChildOutcome> {
    let (_, waiter) = oneshot::channel();
    waiter
}

impl NativeCodingJob {
    /// Stages one batch of helpers and admits it into the job's live session,
    /// opening one on the batch when none is live. Every agent comes back with
    /// its OWN completion waiter, so a `helpers` call returns when its helper
    /// finishes rather than when the whole run does. A batch never fails with
    /// `Busy` because a session is live: it joins the session instead.
    pub async fn delegate(
        self: &Arc<Self>,
        tasks: Vec<CodingTask>,
    ) -> Result<Vec<(AgentId, oneshot::Receiver<ChildOutcome>)>, CodingError> {
        if tasks.is_empty() || tasks.len() > 32 || self.cancellation.is_cancelled() {
            return Err(CodingError::InvalidRequest);
        }
        // Validate, stage and admit are ONE check-then-act over the task
        // history: `validate_delegate_tasks` reads `tasks.jsonl` for the
        // duplicate-agent and `MAX_TASKS` checks, and `stage_children` appends
        // the batch's record only after every child is composed. Holding the
        // one-owner lock across all three is what makes the second `helpers`
        // call of a turn see the first one's record instead of racing it into a
        // duplicate registration. The lock is released before the waiters go
        // back to `task_run`, so a helper's own completion never waits on it.
        let _operation = self.operation.lock().await;
        // `coding.max_children` bounds SIMULTANEOUS children only.
        self.validate_delegate_tasks(&tasks)?;
        let staged = self.stage_delegation(tasks).await?;
        let nodes: Vec<(AgentId, ScheduledChild)> = staged
            .into_iter()
            .map(|child| (child.agent, child.node))
            .collect();
        let launched = nodes.len();
        let waiters = self.admit_children(nodes).await?;
        drop(_operation);
        self.delegated_children
            .fetch_add(launched, std::sync::atomic::Ordering::Release);
        Ok(waiters)
    }

    /// Admits one staged batch: submits it into the live session, or settles a
    /// session that has already stopped and opens a fresh one on the batch.
    async fn admit_children(
        self: &Arc<Self>,
        nodes: Vec<(AgentId, ScheduledChild)>,
    ) -> Result<Vec<(AgentId, oneshot::Receiver<ChildOutcome>)>, CodingError> {
        let mut pending: VecDeque<(AgentId, ScheduledChild)> = nodes.into();
        let mut admitted = Vec::with_capacity(pending.len());
        {
            let mut active = self.active.lock().await;
            if let Some(live) = active.as_mut()
                && let Some(session) = live.session.as_ref()
            {
                let mut controls = BTreeMap::new();
                let mut stopped = false;
                while let Some((agent, node)) = pending.pop_front() {
                    controls.insert(agent.clone(), node.control());
                    match session.submit(node).await {
                        Ok(waiter) => admitted.push((agent, waiter)),
                        Err(ScheduleError::SessionClosed) => {
                            // The session stopped before this child reached it,
                            // so this child never started: its waiter resolves
                            // at once and it is summarized as not started. The
                            // rest of the batch opens a fresh session below.
                            admitted.push((agent, never_started()));
                            stopped = true;
                            break;
                        }
                        Err(error) => return Err(CodingError::owner(error)),
                    }
                }
                if !stopped {
                    live.controls.extend(controls);
                    live.in_flight += admitted.len();
                    return Ok(admitted);
                }
            }
        }
        // No live session, or the live one had already stopped: settle it, then
        // open a session on what is left of the batch. A session already
        // draining (its submit side taken) is not joined again — it is ending.
        let stopped = {
            let active = self.active.lock().await;
            active.as_ref().is_some_and(|live| live.session.is_some())
        };
        if stopped && let Err(error) = self.join().await {
            self.latch_error(&error);
        }
        let remaining: Vec<(AgentId, ScheduledChild)> = pending.into();
        let controls = remaining
            .iter()
            .map(|(agent, node)| (agent.clone(), node.control()))
            .collect();
        let agents: Vec<AgentId> = remaining.iter().map(|(agent, _)| agent.clone()).collect();
        let (live, waiters) = self.open_session(remaining, controls).await?;
        *self.active.lock().await = Some(live);
        admitted.extend(agents.into_iter().zip(waiters));
        Ok(admitted)
    }

    /// Opens a session on the batch and spawns the task that runs it, draining
    /// its `ChildEvent`s into `forward_progress` as the one-shot schedule did.
    /// The job's own cancellation token is the session's caller, so cancelling
    /// the run stops every live helper.
    pub(super) async fn open_session(
        self: &Arc<Self>,
        nodes: Vec<(AgentId, ScheduledChild)>,
        controls: BTreeMap<AgentId, ChildControl>,
    ) -> Result<(LiveSession, Vec<oneshot::Receiver<ChildOutcome>>), CodingError> {
        let limits = self.ledger.limits().await;
        let maximum = usize::try_from(limits.child_slots.min(limits.model_slots.saturating_sub(1)))
            .unwrap_or(0);
        let (sender, mut receiver) = mpsc::channel(128);
        let (session, run, waiters) = ScheduleSession::open(
            nodes.into_iter().map(|(_, node)| node).collect(),
            ScheduleLimits {
                max_active_children: maximum,
                max_active_per_provider: maximum,
            },
            &self.ledger,
            self.cancellation.clone(),
            sender,
        )
        .await
        .map_err(CodingError::owner)?;
        let owner = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let run = run.run();
            tokio::pin!(run);
            loop {
                tokio::select! {
                    result = &mut run => {
                        while let Ok(event) = receiver.try_recv() {
                            owner.forward_progress(event);
                        }
                        return result.map_err(CodingError::owner);
                    }
                    Some(event) = receiver.recv() => owner.forward_progress(event),
                }
            }
        });
        let in_flight = waiters.len();
        Ok((
            LiveSession {
                session: Some(session),
                handle: Some(handle),
                controls,
                in_flight,
            },
            waiters,
        ))
    }

    /// Settles the live session: drops the submit side (which closes the
    /// session, so its running half drains), awaits the task running it, then
    /// normalizes and latches the report as the join always did. Shared by the
    /// per-helper release, `settle_implicit_join` and `finalize_owned`;
    /// `active` is `None` once it returns.
    pub async fn join(&self) -> Result<ScheduleReport, CodingError> {
        let handle = {
            let mut active = self.active.lock().await;
            let live = active.as_mut().ok_or(CodingError::NotOwned)?;
            // Closing the session first lets it drain while the controls stay
            // readable for a child that is still running.
            live.session = None;
            live.handle.take().ok_or(CodingError::NotOwned)?
        };
        let outcome = handle
            .await
            .map_err(CodingError::owner)
            .and_then(|report| report);
        self.active.lock().await.take();
        let mut report = match outcome {
            Ok(report) => report,
            Err(error) => {
                self.latch_error(&error);
                return Err(error);
            }
        };
        if let Err(error) = self.normalize_join_accounting(&mut report).await {
            self.latch_error(&error);
            return Err(error);
        }
        self.latch_schedule(&report);
        Ok(report)
    }

    /// Releases this call's helpers from the live session. The session is
    /// settled once no helper is left in flight, so it never outlives the
    /// helpers it runs.
    pub(super) async fn release_helpers(&self, count: usize) {
        let _operation = self.operation.lock().await;
        let settled = {
            let mut active = self.active.lock().await;
            match active.as_mut() {
                Some(live) => {
                    live.in_flight = live.in_flight.saturating_sub(count);
                    live.in_flight == 0
                }
                None => false,
            }
        };
        if settled && let Err(error) = self.join().await {
            self.latch_error(&error);
        }
    }

    pub async fn message(&self, agent: &AgentId, text: &str) -> Result<(), CodingError> {
        self.control(agent).await?.send(text).map_err(control_error)
    }
    pub async fn pause(&self, agent: &AgentId) -> Result<u64, CodingError> {
        self.control(agent).await?.pause().map_err(control_error)
    }
    pub async fn resume_child(&self, agent: &AgentId) -> Result<(), CodingError> {
        self.control(agent).await?.resume().map_err(control_error)
    }
    pub async fn cancel_child(&self, agent: &AgentId) -> Result<(), CodingError> {
        self.control(agent).await?.cancel();
        Ok(())
    }
    pub(super) async fn control(&self, agent: &AgentId) -> Result<ChildControl, CodingError> {
        self.active
            .lock()
            .await
            .as_ref()
            .and_then(|live| live.controls.get(agent))
            .cloned()
            .ok_or(CodingError::NotOwned)
    }
}
