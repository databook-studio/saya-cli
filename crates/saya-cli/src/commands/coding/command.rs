use super::{CodingError, checks::ChildCodingOperations};
use saya_agent::CancellationToken;
use saya_harness::{
    coding::{
        commands::{CommandOutputCursor, CommandSession, OutputStream},
        worktree::{confine_checkout, verify_owned_checkout},
    },
    runner::StaticCredentialSource,
};
use serde_json::{Value, json};
use std::{num::NonZeroU64, sync::Arc, time::Duration};

pub(crate) struct ActiveCommand {
    id: u64,
    session: Option<CommandSession>,
    joining: Option<
        tokio::task::JoinHandle<
            Result<
                saya_harness::runner::ProgramOutcome,
                saya_harness::coding::commands::CommandSessionError,
            >,
        >,
    >,
    stdout: CommandOutputCursor,
    stderr: CommandOutputCursor,
}
impl ChildCodingOperations {
    pub(crate) async fn start(
        &self,
        program: &str,
        argv: Vec<String>,
        timeout: u64,
        token: &CancellationToken,
    ) -> Result<Value, CodingError> {
        self.authority
            .admit_runner_program(program)
            .map_err(CodingError::owner)?;
        if timeout == 0 || timeout > 900 {
            return Err(CodingError::InvalidRequest);
        }
        let mut sessions = self.sessions.lock().await;
        if sessions.len() >= 2 {
            return Err(CodingError::Busy);
        }
        let verified = verify_owned_checkout(&self.ownership(), token)
            .await
            .map_err(CodingError::owner)?;
        let confined = confine_checkout(&verified, &self.application.program_dir)
            .map_err(CodingError::owner)?;
        let runner = confined
            .runner_tool(
                self.authority.capabilities().runner.clone(),
                None,
                Duration::from_secs(timeout),
                Arc::new(StaticCredentialSource::new([])),
                self.records.clone(),
            )
            .with_cancellation(token.clone());
        let session = self
            .owner
            .start_with_output(
                runner,
                program.into(),
                argv,
                NonZeroU64::new(timeout).ok_or(CodingError::InvalidRequest)?,
                self.records.clone(),
            )
            .map_err(CodingError::owner)?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        sessions.push(ActiveCommand {
            id,
            session: Some(session),
            joining: None,
            stdout: CommandOutputCursor::start(OutputStream::Stdout),
            stderr: CommandOutputCursor::start(OutputStream::Stderr),
        });
        Ok(json!({"command_id":id}))
    }
    pub(crate) async fn read(&self, id: u64, stderr: bool) -> Result<Value, CodingError> {
        let mut sessions = self.sessions.lock().await;
        let active = sessions
            .iter_mut()
            .find(|active| active.id == id)
            .ok_or(CodingError::NotOwned)?;
        let cursor = if stderr {
            &mut active.stderr
        } else {
            &mut active.stdout
        };
        let read = active
            .session
            .as_ref()
            .ok_or(CodingError::Busy)?
            .read_output(*cursor, 4096)
            .map_err(CodingError::owner)?;
        *cursor = read.next_cursor();
        Ok(
            json!({"text":read.text(),"truncated":read.truncated(),"end_of_stream":read.end_of_stream(),"next_offset":read.next_cursor().offset()}),
        )
    }
    pub(crate) async fn stop(&self, id: u64) -> Result<Value, CodingError> {
        let mut sessions = self.sessions.lock().await;
        let index = sessions
            .iter()
            .position(|active| active.id == id)
            .ok_or(CodingError::NotOwned)?;
        let active = &mut sessions[index];
        if let Some(session) = active.session.take() {
            active.joining = Some(tokio::spawn(session.stop_and_join()));
        }
        // The handle stays in the owner while awaiting, so cancellation of
        // this call cannot detach an untracked session.
        let result = active.joining.as_mut().ok_or(CodingError::NotOwned)?.await;
        active.joining.take();
        sessions.remove(index);
        let outcome = match result {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => {
                self.failed
                    .store(true, std::sync::atomic::Ordering::Release);
                return Err(CodingError::owner(error));
            }
            Err(error) => {
                self.failed
                    .store(true, std::sync::atomic::Ordering::Release);
                return Err(CodingError::owner(error));
            }
        };
        if outcome.killed_orphans || (outcome.exit_code.is_none() && outcome.signal.is_none()) {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        super::context::append_line(&self.records.join(format!("commands-{}.jsonl",self.checkout.as_str())),&serde_json::to_vec(&json!({"job_id":self.job,"checkout_id":self.checkout,"program":outcome.program,"pid":outcome.pid,"exit_code":outcome.exit_code,"signal":outcome.signal,"timed_out":outcome.killed_by_timeout,"cancelled":outcome.cancelled,"killed_orphans":outcome.killed_orphans,"joined":true})).map_err(CodingError::owner)?,1024*1024)?;
        Ok(
            json!({"exit_code":outcome.exit_code,"cancelled":outcome.cancelled,"timed_out":outcome.killed_by_timeout,"signal":outcome.signal,"killed_orphans":outcome.killed_orphans}),
        )
    }
    pub(crate) async fn settle(&self) -> Result<(), CodingError> {
        let ids = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|active| active.id)
            .collect::<Vec<_>>();
        let mut failure = None;
        for id in ids {
            match self.stop(id).await {
                Ok(value) if value.get("killed_orphans").and_then(Value::as_bool) == Some(true) => {
                    failure = Some(CodingError::NotOwned)
                }
                Ok(_) => {}
                Err(error) => failure = Some(error),
            }
        }
        if self.failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(CodingError::NotOwned);
        }
        failure.map_or(Ok(()), Err)
    }
}
