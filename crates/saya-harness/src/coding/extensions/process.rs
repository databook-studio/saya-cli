use super::{
    ExtensionConfig, ExtensionError, ExtensionProcessEvent, ExtensionProcessObserver,
    ExtensionProcessReceipt,
    framing::{self, Framing},
};
use saya_agent::CancellationToken;
use serde_json::{Value, json};
#[cfg(unix)]
use std::process::Stdio;
use std::sync::Arc;
#[cfg(unix)]
use std::sync::LazyLock;
use std::time::Duration;
#[cfg(unix)]
static SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
use tokio::{
    io::BufReader,
    process::{Child, ChildStdin, ChildStdout},
};

pub(super) struct ProtocolProcess {
    child: Option<Child>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    observer: Option<Arc<dyn ExtensionProcessObserver>>,
    fork_denied: bool,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    framing: Framing,
    next_id: u64,
    messages: usize,
    pub(super) notifications: Vec<Value>,
    secrets: Vec<String>,
    pid: u32,
    receipt: Option<ExtensionProcessReceipt>,
    /// Mandatory reaping/observer failure. Once set it is sticky: later
    /// callers must not mistake an already-reaped process for a successful
    /// durable settlement.
    settlement_failure: Option<ExtensionError>,
}
impl ProtocolProcess {
    pub(super) fn start(config: ExtensionConfig, framing: Framing) -> Result<Self, ExtensionError> {
        #[cfg(not(unix))]
        {
            let _ = (config, framing);
            Err(ExtensionError::Unsupported)
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let permit = Arc::clone(&SLOTS)
                .try_acquire_owned()
                .map_err(|_| ExtensionError::Limit)?;
            let mut command = config.spawn.command(&config.executable);
            command
                .args(&config.argv)
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0);
            let mut secrets = Vec::new();
            for credential in &config.credentials {
                let secret = credential
                    .resolved(config.resolver.as_ref())
                    .map_err(|_| ExtensionError::Configuration)?;
                if secret.is_empty() || secret.len() > 8192 {
                    return Err(ExtensionError::Configuration);
                }
                command.env(credential.env_var(), &secret);
                secrets.push(secret);
            }
            let mut command = tokio::process::Command::from(command);
            command.kill_on_drop(true);
            let mut child = command.spawn().map_err(|_| ExtensionError::Process)?;
            let pid = child.id().ok_or(ExtensionError::Process)?;
            let input = child.stdin.take().ok_or(ExtensionError::Process)?;
            let output = BufReader::new(child.stdout.take().ok_or(ExtensionError::Process)?);
            let fork_denied = config.spawn.profile().is_some_and(|profile| {
                !profile
                    .lines()
                    .any(|line| line.trim_start().starts_with("(allow process-fork)"))
            });
            let process = Self {
                child: Some(child),
                permit: Some(permit),
                observer: config.observer,
                fork_denied,
                input,
                output,
                framing,
                next_id: 1,
                messages: 0,
                notifications: Vec::new(),
                secrets,
                pid,
                receipt: None,
                settlement_failure: None,
            };
            if let Some(observer) = &process.observer {
                observer.record(ExtensionProcessEvent::Spawned { pid })?;
            }
            Ok(process)
        }
    }

    pub(super) async fn notify(
        &mut self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<(), ExtensionError> {
        let value = json!({"jsonrpc":"2.0", "method":method, "params":params});
        let result = tokio::select! {
            _ = cancel.cancelled() => Err(ExtensionError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(10), framing::write(&mut self.input, self.framing, &value)) => result.unwrap_or(Err(ExtensionError::Timeout)),
        };
        if result.is_err() {
            // The protocol error remains the caller's primary result; this
            // records any later mandatory settlement failure for shutdown.
            let _ = self.shutdown().await;
        }
        result
    }

    pub(super) async fn request(
        &mut self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, ExtensionError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(ExtensionError::Limit)?;
        let request = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
        let result = tokio::select! {
            _ = cancel.cancelled() => Err(ExtensionError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(10), self.exchange(request, id)) => result.unwrap_or(Err(ExtensionError::Timeout)),
        };
        if result.is_err() {
            // Do not use `?`: protocol exchange precedence is preserved while
            // the cleanup owner retains a typed sticky settlement failure.
            let _ = self.shutdown().await;
        }
        result
    }

    async fn exchange(&mut self, request: Value, id: u64) -> Result<Value, ExtensionError> {
        framing::write(&mut self.input, self.framing, &request).await?;
        loop {
            let mut value = self.receive().await?;
            if value.get("id").is_some() {
                if value.get("id").and_then(Value::as_u64) != Some(id)
                    || value.get("error").is_some()
                    || value.get("result").is_none()
                {
                    return Err(ExtensionError::Protocol);
                }
                return Ok(value["result"].take());
            }
            if self.notifications.len() >= 16 {
                return Err(ExtensionError::Limit);
            }
            self.notifications.push(value);
        }
    }

    pub(super) async fn receive(&mut self) -> Result<Value, ExtensionError> {
        self.messages += 1;
        if self.messages > 128 {
            return Err(ExtensionError::Limit);
        }
        let mut value = framing::read(&mut self.output, self.framing).await?;
        scrub(&mut value, &self.secrets);
        Ok(value)
    }

    /// Kill the owned group on every exit path, then reap the leader.
    pub(super) async fn shutdown(&mut self) -> Result<(), ExtensionError> {
        self.shutdown_receipt().await.map(|_| ())
    }

    pub(super) async fn shutdown_receipt(
        &mut self,
    ) -> Result<ExtensionProcessReceipt, ExtensionError> {
        if let Some(error) = self.settlement_failure {
            return Err(error);
        }
        if let Some(receipt) = &self.receipt {
            return Ok(receipt.clone());
        }
        let delivered = self.kill_group();
        let Some(child) = self.child.take() else {
            return Err(self.remember_settlement_failure(ExtensionError::Process));
        };
        let receipt =
            match super::receipt::settle(child, self.pid, delivered, self.fork_denied).await {
                Ok(receipt) => receipt,
                Err(error) => {
                    let observed = self.record_uncertain();
                    return Err(self.remember_settlement_failure(observed.unwrap_or(error)));
                }
            };
        let observer_failed = self.observer.as_ref().is_some_and(|observer| {
            observer
                .record(ExtensionProcessEvent::Settled(receipt.clone()))
                .is_err()
        });
        if observer_failed {
            return Err(self.remember_settlement_failure(ExtensionError::Observer));
        }
        // Only a successfully published mandatory settled receipt may be
        // cached as success.
        self.receipt = Some(receipt.clone());
        Ok(receipt)
    }

    pub(super) fn settlement_failure(&self) -> Option<ExtensionError> {
        self.settlement_failure
    }

    pub(super) fn has_settled_receipt(&self) -> bool {
        self.receipt.is_some()
    }

    fn remember_settlement_failure(&mut self, error: ExtensionError) -> ExtensionError {
        *self.settlement_failure.get_or_insert(error)
    }

    fn record_uncertain(&self) -> Option<ExtensionError> {
        self.observer.as_ref().and_then(|observer| {
            observer
                .record(ExtensionProcessEvent::Uncertain { pid: self.pid })
                .err()
                .map(|_| ExtensionError::Observer)
        })
    }
    fn kill_group(&self) -> bool {
        #[cfg(unix)]
        if self.child.is_some() {
            // The child became this group's leader at spawn; never a caller PID.
            return unsafe { libc::killpg(self.pid as libc::pid_t, libc::SIGKILL) == 0 };
        }
        false
    }
}
impl Drop for ProtocolProcess {
    fn drop(&mut self) {
        let delivered = self.kill_group();
        let Some(child) = self.child.take() else {
            return;
        };
        let observer = self.observer.clone();
        let permit = self.permit.take();
        let pid = self.pid;
        let fork_denied = self.fork_denied;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _permit = permit;
                let result = super::receipt::settle(child, pid, delivered, fork_denied).await;
                if let Some(observer) = observer {
                    let event = result
                        .map(ExtensionProcessEvent::Settled)
                        .unwrap_or(ExtensionProcessEvent::Uncertain { pid });
                    let _ = observer.record(event);
                }
            });
        } else if let Some(observer) = observer {
            let _ = observer.record(ExtensionProcessEvent::Uncertain { pid });
        }
    }
}

fn scrub(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            for secret in secrets {
                *text = text.replace(secret, "[REDACTED]");
            }
            *text = saya_types::redact(text);
        }
        Value::Array(items) => items.iter_mut().for_each(|item| scrub(item, secrets)),
        Value::Object(map) => {
            let old = std::mem::take(map);
            for (key, mut item) in old {
                let mut key = Value::String(key);
                scrub(&mut key, secrets);
                scrub(&mut item, secrets);
                if let Value::String(key) = key {
                    map.insert(key, item);
                }
            }
        }
        Value::Number(number)
            if secrets
                .iter()
                .any(|secret| number.to_string().contains(secret)) =>
        {
            *value = Value::String("[REDACTED]".into());
        }
        _ => {}
    }
}
