use super::{CodingMessage, CodingPanel};
use crate::{
    cli::CodingCommand,
    commands::coding::{
        CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, dispatch_coding,
    },
    config::runtime::RuntimeConfig,
};
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, CancellationToken, NoopEventSink, ToolDefinition};
use saya_store::SqliteStateStore;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};
#[cfg(test)]
mod worker_tests;
struct PanelApproval {
    sender: mpsc::SyncSender<CodingMessage>,
    token: CancellationToken,
    id: String,
}
#[async_trait]
impl ApprovalDecider for PanelApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        // The panel answers for exactly one job: a proposal that names any
        // other job (or none) is never put to the operator as this job's.
        if tool.name != "coding_job"
            || arguments.get("job_id").and_then(Value::as_str) != Some(&self.id)
        {
            return false;
        }
        let (respond, answer) = tokio::sync::oneshot::channel();
        let text = format!(
            "Approve native coding job {} with this exact scope:\n{}\nUse /coding approve {} or /coding deny {}.",
            self.id, arguments, self.id, self.id
        );
        if self
            .sender
            .try_send(CodingMessage::Approval { text, respond })
            .is_err()
        {
            return false;
        }
        tokio::select! { _=self.token.cancelled()=>false, answer=tokio::time::timeout(std::time::Duration::from_secs(120),answer)=>matches!(answer,Ok(Ok(true))) }
    }
}
pub(super) fn spawn(
    command: CodingCommand,
    runtime: RuntimeConfig,
    store: SqliteStateStore,
    repository: PathBuf,
) -> CodingPanel {
    let (tx, rx) = mpsc::sync_channel(128);
    let (receipt_tx, receipts) = mpsc::sync_channel(64);
    let (done_tx, done) = mpsc::sync_channel(1);
    let (commands, control) = tokio::sync::mpsc::channel(16);
    let cancellation = CancellationToken::new();
    let token = cancellation.clone();
    let id = match &command {
        CodingCommand::Start { id, .. } => Some(id.clone()),
        _ => None,
    };
    let worker = std::thread::spawn(move || {
        let senders = PanelSenders {
            activity: tx,
            receipts: receipt_tx,
        };
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|runtime_owner| {
                runtime_owner
                    .block_on(work(
                        command, runtime, store, repository, token, senders, control,
                    ))
                    .map_err(|error| error.to_string())
            });
        let _ = done_tx.send(result);
    });
    CodingPanel {
        id,
        rx,
        receipts,
        done,
        commands,
        cancellation,
        pending: None,
        worker: Some(worker),
        active: true,
    }
}
/// Operator-visible outcomes (control receipts, pause acknowledgements) ride
/// their own bounded queue and the send blocks until it has room: a dropped
/// receiver means the panel is gone, so the send is ignored. They must not
/// share the activity queue — a same-tick burst of progress events pushed
/// behind a receipt scrolls it above the viewport before any frame draws it,
/// and the receipt is never painted.
fn deliver_outcome(receipts: &mpsc::SyncSender<CodingMessage>, text: String) {
    let _ = receipts.send(CodingMessage::Activity(text));
}
struct PanelSenders {
    activity: mpsc::SyncSender<CodingMessage>,
    receipts: mpsc::SyncSender<CodingMessage>,
}
async fn work(
    command: CodingCommand,
    runtime: RuntimeConfig,
    store: SqliteStateStore,
    repository: PathBuf,
    token: CancellationToken,
    senders: PanelSenders,
    mut controls: tokio::sync::mpsc::Receiver<CodingCommand>,
) -> Result<String, CodingError> {
    let PanelSenders {
        activity: tx,
        receipts,
    } = senders;
    let application = CodingApplication::session(runtime, store, &repository, &token).await?;
    let CodingCommand::Start {
        id,
        task,
        write_paths,
        programs,
        approve,
    } = command
    else {
        let shown = command.clone();
        return dispatch_coding(&application, command)
            .await
            .map(|value| crate::render::render_coding_result(&shown, &value));
    };
    let request = CodingJobRequest::new(
        saya_types::JobId::parse(&id).map_err(CodingError::owner)?,
        task,
        write_paths,
        programs,
    )?;
    let approval: Box<dyn ApprovalDecider> = if approve {
        Box::new(CodingAuthorization::scoped(
            &application.propose(&request).await?,
        )?)
    } else {
        Box::new(PanelApproval {
            sender: tx.clone(),
            token: token.clone(),
            id,
        })
    };
    let job = application
        .create(request, approval.as_ref(), token)
        .await?;
    let mut progress = job.subscribe();
    let pause_stop = CancellationToken::new();
    let mut pause_acks = tokio::task::JoinSet::new();
    let run = job.run(&NoopEventSink);
    tokio::pin!(run);
    loop {
        tokio::select! {
            result=&mut run=>{
                pause_stop.cancel();
                while pause_acks.join_next().await.is_some() {}
                return result.map(|output|output.answer)
            },
            Some(command)=controls.recv()=>{
                let pause = match &command {
                    CodingCommand::Pause { id, agent } => Some((id.clone(), agent.clone())),
                    _ => None,
                };
                let result=job.control_command(command).await;
                if let (Ok(receipt), Some((id, agent))) = (&result, pause)
                    && let Some(ticket) = receipt["pause_requested"].as_u64()
                {
                    let job = Arc::clone(&job);
                    let receipts = receipts.clone();
                    let stop = pause_stop.clone();
                    pause_acks.spawn(async move {
                        let acknowledged = tokio::select! {
                            result = job.wait_pause(&id, &agent, ticket) => result.unwrap_or(false),
                            _ = stop.cancelled() => false,
                        };
                        deliver_outcome(
                            &receipts,
                            json!({"pause_requested":ticket,"pause_acknowledged":acknowledged,"target":agent})
                                .to_string(),
                        );
                    });
                }
                let text=match result {Ok(value)=>value.to_string(),Err(error)=>error.to_string()};
                deliver_outcome(&receipts, text);
            },
            result = pause_acks.join_next(), if !pause_acks.is_empty() => {
                let _ = result;
            },
            progress=progress.recv()=>{
                let text=match progress {Ok(event)=>serde_json::to_string(&event).map_err(CodingError::owner)?,Err(tokio::sync::broadcast::error::RecvError::Lagged(count))=>format!("{count} activity updates omitted; durable results remain available"),Err(_)=>continue};
                let _=tx.try_send(CodingMessage::Activity(text));
            },
        }
    }
}
