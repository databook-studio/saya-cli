use super::{
    ExtensionConfig, ExtensionError,
    framing::{self, Framing},
};
use saya_agent::CancellationToken;
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::{Arc, LazyLock},
    time::Duration,
};
static SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
use tokio::{
    io::BufReader,
    process::{Child, ChildStdin, ChildStdout},
};

pub(super) struct ProtocolProcess {
    child: Option<Child>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    framing: Framing,
    next_id: u64,
    messages: usize,
    pub(super) notifications: Vec<Value>,
    secrets: Vec<String>,
    pid: u32,
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
            Ok(Self {
                child: Some(child),
                _permit: permit,
                input,
                output,
                framing,
                next_id: 1,
                messages: 0,
                notifications: Vec::new(),
                secrets,
                pid,
            })
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
            self.shutdown().await?;
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
            self.shutdown().await?;
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

    /// Kill the complete owned group on every exit path, then reap the leader.
    pub(super) async fn shutdown(&mut self) -> Result<(), ExtensionError> {
        self.kill_group();
        if let Some(mut child) = self.child.take() {
            if child
                .try_wait()
                .map_err(|_| ExtensionError::Process)?
                .is_none()
            {
                child.start_kill().map_err(|_| ExtensionError::Process)?;
            }
            tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .map_err(|_| ExtensionError::Timeout)?
                .map_err(|_| ExtensionError::Process)?;
        }
        Ok(())
    }
    fn kill_group(&self) {
        #[cfg(unix)]
        if self.child.is_some() {
            // The child became this group's leader at spawn; never a caller PID.
            unsafe {
                libc::killpg(self.pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}
impl Drop for ProtocolProcess {
    fn drop(&mut self) {
        self.kill_group();
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
