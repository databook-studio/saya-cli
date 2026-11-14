//! One control/progress driver for initial and follow-up native runs.
use super::{CodingMessage, PanelSenders, deliver_outcome};
use crate::{
    cli::CodingCommand,
    commands::coding::{CodingError, NativeCodingJob},
};
use saya_agent::CancellationToken;
use serde_json::json;
use std::{future::Future, sync::Arc};
pub(super) async fn drive<F>(
    job: &Arc<NativeCodingJob>,
    run: F,
    senders: PanelSenders,
    mut controls: tokio::sync::mpsc::Receiver<CodingCommand>,
) -> Result<String, CodingError>
where
    F: Future<Output = Result<String, CodingError>>,
{
    let PanelSenders {
        activity: tx,
        receipts,
    } = senders;
    let mut progress = job.subscribe();
    let pause_stop = CancellationToken::new();
    let mut pause_acks = tokio::task::JoinSet::new();
    tokio::pin!(run);
    loop {
        tokio::select! {
            result=&mut run=>{
                pause_stop.cancel();
                while pause_acks.join_next().await.is_some() {}
                return result
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
                    let job = Arc::clone(job);
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
