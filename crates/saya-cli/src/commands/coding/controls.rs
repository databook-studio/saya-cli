use super::{CodingError, NativeCodingJob, delegate::CodingTask};
use saya_harness::coding::{
    child::ChildControl,
    coordinator::{ScheduleLimits, ScheduleReport, run_schedule},
};
use saya_types::AgentId;
use std::{collections::BTreeMap, sync::Arc};
use tokio::{sync::mpsc, task::JoinHandle};

pub(super) struct LiveSchedule {
    controls: BTreeMap<AgentId, ChildControl>,
    handle: Arc<tokio::sync::Mutex<Option<JoinHandle<Result<ScheduleReport, CodingError>>>>>,
}
impl NativeCodingJob {
    pub async fn delegate(
        self: &Arc<Self>,
        tasks: Vec<CodingTask>,
    ) -> Result<Vec<AgentId>, CodingError> {
        if tasks.is_empty() || tasks.len() > 32 || self.cancellation.is_cancelled() {
            return Err(CodingError::InvalidRequest);
        }
        let mut active = self.active.lock().await;
        if active.is_some() {
            return Err(CodingError::Busy);
        }
        super::context::append_line(
            &self.records.join("tasks.jsonl"),
            &serde_json::to_vec(&tasks).map_err(CodingError::owner)?,
            1024 * 1024,
        )?;
        let mut nodes = Vec::new();
        let mut controls = BTreeMap::new();
        for task in tasks {
            let agent = task.agent_id.clone();
            if controls.contains_key(&agent) {
                return Err(CodingError::InvalidRequest);
            }
            let node = self.prepare_child(task).await?;
            controls.insert(agent, node.control());
            nodes.push(node);
        }
        let agents = controls.keys().cloned().collect();
        let owner = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let config = &owner.application.runtime.resolved.coding;
            let maximum = config
                .max_children
                .min(config.max_concurrent_model_requests.saturating_sub(1));
            let (sender, mut receiver) = mpsc::channel(128);
            let run = run_schedule(
                nodes,
                ScheduleLimits {
                    max_active_children: maximum,
                    max_active_per_provider: maximum,
                },
                &owner.ledger,
                owner.cancellation.clone(),
                sender,
            );
            tokio::pin!(run);
            loop {
                tokio::select! {
                    result=&mut run => {
                        while let Ok(event)=receiver.try_recv() { owner.forward_progress(event); }
                        return result.map_err(CodingError::owner);
                    },
                    Some(event)=receiver.recv() => { owner.forward_progress(event); }
                }
            }
        });
        *active = Some(LiveSchedule {
            controls,
            handle: Arc::new(tokio::sync::Mutex::new(Some(handle))),
        });
        Ok(agents)
    }
    pub async fn join(&self) -> Result<ScheduleReport, CodingError> {
        let slot = self
            .active
            .lock()
            .await
            .as_ref()
            .map(|active| Arc::clone(&active.handle))
            .ok_or(CodingError::NotOwned)?;
        let mut slot = slot.lock().await;
        let outcome = slot
            .as_mut()
            .ok_or(CodingError::NotOwned)?
            .await
            .map_err(CodingError::owner);
        slot.take();
        self.active.lock().await.take();
        let report = outcome??;
        if report.stop.is_some() || report.effects_uncertain {
            self.child_failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(report)
    }

    pub async fn message(&self, agent: &AgentId, text: &str) -> Result<(), CodingError> {
        self.control(agent)
            .await?
            .send(text)
            .map_err(CodingError::owner)
    }
    pub async fn pause(&self, agent: &AgentId) -> Result<u64, CodingError> {
        self.control(agent)
            .await?
            .pause()
            .map_err(CodingError::owner)
    }
    pub async fn resume_child(&self, agent: &AgentId) -> Result<(), CodingError> {
        self.control(agent)
            .await?
            .resume()
            .map_err(CodingError::owner)
    }
    pub async fn cancel_child(&self, agent: &AgentId) -> Result<(), CodingError> {
        self.control(agent).await?.cancel();
        Ok(())
    }
    async fn control(&self, agent: &AgentId) -> Result<ChildControl, CodingError> {
        self.active
            .lock()
            .await
            .as_ref()
            .and_then(|active| active.controls.get(agent))
            .cloned()
            .ok_or(CodingError::NotOwned)
    }
}

impl NativeCodingJob {
    pub async fn control_command(
        &self,
        command: crate::cli::CodingCommand,
    ) -> Result<serde_json::Value, CodingError> {
        use crate::cli::CodingCommand;
        let (id, agent) = match &command {
            CodingCommand::Message { id, agent, .. }
            | CodingCommand::Pause { id, agent }
            | CodingCommand::Continue { id, agent }
            | CodingCommand::CancelChild { id, agent } => (id, agent),
            _ => return Err(CodingError::InvalidRequest),
        };
        if id != self.job_id().as_str() {
            return Err(CodingError::NotOwned);
        }
        if agent == "coordinator" {
            return match command {
                CodingCommand::Message { text, .. } => {
                    self.root_steering.send(&text).map_err(CodingError::owner)?;
                    Ok(serde_json::json!({"queued":true,"target":"coordinator"}))
                }
                CodingCommand::Pause { .. } => {
                    let ticket = self.root_steering.pause().map_err(CodingError::owner)?;
                    Ok(
                        serde_json::json!({"pause_requested":ticket,"acknowledged":false,"target":"coordinator"}),
                    )
                }
                CodingCommand::Continue { .. } => {
                    self.root_steering.resume().map_err(CodingError::owner)?;
                    Ok(serde_json::json!({"resumed":true,"target":"coordinator"}))
                }
                _ => Err(CodingError::InvalidRequest),
            };
        }
        let agent = saya_types::AgentId::parse(agent).map_err(CodingError::owner)?;
        match command {
            CodingCommand::Message { text, .. } => {
                self.message(&agent, &text).await?;
                Ok(serde_json::json!({"queued":true}))
            }
            CodingCommand::Pause { .. } => {
                let ticket = self.pause(&agent).await?;
                Ok(serde_json::json!({"pause_requested":ticket,"acknowledged":false}))
            }
            CodingCommand::Continue { .. } => {
                self.resume_child(&agent).await?;
                Ok(serde_json::json!({"resumed":true}))
            }
            CodingCommand::CancelChild { .. } => {
                self.cancel_child(&agent).await?;
                Ok(serde_json::json!({"cancel_requested":true}))
            }
            _ => Err(CodingError::InvalidRequest),
        }
    }
}
