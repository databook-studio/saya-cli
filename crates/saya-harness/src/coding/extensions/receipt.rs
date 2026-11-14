//! Process facts observed by this extension owner, never supplied by a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionProcessReceipt {
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// The direct child was waited and reaped by this owner.
    pub joined: bool,
    /// A group-wide SIGKILL was requested before waiting, including normal exit.
    pub group_kill_requested: bool,
    pub group_kill_delivered: bool,
    /// Process-group cleanup cannot prove absence of descendants that changed
    /// session/group. Never relabel this as a host-wide absence proof.
    pub escaped_descendants_proven_absent: bool,
}

/// Host-owned bounded persistence hook. Protocol text never selects this sink.
pub trait ExtensionProcessObserver: Send + Sync {
    fn record(&self, event: ExtensionProcessEvent) -> Result<(), super::ExtensionError>;
}
#[derive(Debug, Clone)]
pub enum ExtensionProcessEvent {
    Spawned { pid: u32 },
    Settled(ExtensionProcessReceipt),
    Uncertain { pid: u32 },
}

pub(super) async fn settle(
    mut child: tokio::process::Child,
    pid: u32,
    delivered: bool,
    fork_denied: bool,
) -> Result<ExtensionProcessReceipt, super::ExtensionError> {
    use super::ExtensionError;
    if child
        .try_wait()
        .map_err(|_| ExtensionError::Process)?
        .is_none()
    {
        child.start_kill().map_err(|_| ExtensionError::Process)?;
    }
    let status = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
        .await
        .map_err(|_| ExtensionError::Timeout)?
        .map_err(|_| ExtensionError::Process)?;
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    Ok(ExtensionProcessReceipt {
        pid,
        exit_code: status.code(),
        signal,
        joined: true,
        group_kill_requested: true,
        group_kill_delivered: delivered,
        escaped_descendants_proven_absent: fork_denied,
    })
}
