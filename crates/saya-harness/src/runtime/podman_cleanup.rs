//! Cleanup proves the fixed probe name and ticket before removing a container.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;

use super::{
    CleanupStatus,
    podman_process::{read_container_id, run},
};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const CLEANUP_BUDGET: Duration = Duration::from_secs(30);
const LABEL_QUERY: &str =
    "{{.Id}}\t{{.Name}}\t{{index .Config.Labels \"io.saya.managed-node-ticket\"}}";

pub(super) struct CleanupContext<'a> {
    executable: &'a Path,
    home: Option<&'a Path>,
    authfile: &'a Path,
    deadline: Instant,
    cancellation: &'a CancellationToken,
}

impl<'a> CleanupContext<'a> {
    pub(super) fn new(
        executable: &'a Path,
        home: Option<&'a Path>,
        authfile: &'a Path,
        deadline: Instant,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            executable,
            home,
            authfile,
            deadline,
            cancellation,
        }
    }
}

pub(super) async fn cleanup(
    context: &CleanupContext<'_>,
    cidfile: &Path,
    name: &str,
    ticket: &str,
    completed: bool,
) -> CleanupStatus {
    let id = read_container_id(cidfile).ok();
    if let Some(id) = id.as_deref() {
        match inspect_owned(context, id, name, ticket).await {
            Ok(Some(owned)) => {
                return remove_owned(context, &owned).await;
            }
            Ok(None) => {}
            Err(()) => return CleanupStatus::Unknown,
        }
    }
    match inspect_owned(context, name, name, ticket).await {
        Ok(Some(owned)) => remove_owned(context, &owned).await,
        Ok(None) if completed => CleanupStatus::NotNeeded,
        Ok(None) => CleanupStatus::Unknown,
        Err(()) => CleanupStatus::Unknown,
    }
}

async fn inspect_owned(
    context: &CleanupContext<'_>,
    locator: &str,
    expected_name: &str,
    ticket: &str,
) -> Result<Option<String>, ()> {
    if !exists(context, locator).await? {
        return Ok(None);
    }
    let output = run_bounded(
        context,
        &["container", "inspect", "--format", LABEL_QUERY, locator],
    )
    .await
    .map_err(|_| ())?;
    if !output.success {
        return Err(());
    }
    let mut fields = output.stdout.text.trim().split('\t');
    let id = fields.next().ok_or(())?.trim();
    let name = fields.next().ok_or(())?.trim().trim_start_matches('/');
    let label = fields.next().ok_or(())?.trim();
    if fields.next().is_some()
        || !valid_container_id(id)
        || name != expected_name
        || label != ticket
    {
        return Err(());
    }
    Ok(Some(id.to_owned()))
}

async fn remove_owned(context: &CleanupContext<'_>, id: &str) -> CleanupStatus {
    let removed = run_bounded(context, &["rm", "--force", id]).await;
    if !matches!(removed, Ok(output) if output.success) {
        return CleanupStatus::Unknown;
    }
    if matches!(exists(context, id).await, Ok(false)) {
        CleanupStatus::Removed
    } else {
        CleanupStatus::Unknown
    }
}

async fn exists(context: &CleanupContext<'_>, locator: &str) -> Result<bool, ()> {
    let output = run_bounded(context, &["container", "exists", locator])
        .await
        .map_err(|_| ())?;
    match output.exit_code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(()),
    }
}

async fn run_bounded(
    context: &CleanupContext<'_>,
    arguments: &[&str],
) -> Result<super::podman_process::Output, ()> {
    if context.cancellation.is_cancelled() {
        return Err(());
    }
    let timeout = context.deadline.saturating_duration_since(Instant::now());
    if timeout.is_zero() {
        return Err(());
    }
    let output = run(
        context.executable,
        context.home,
        context.authfile,
        arguments,
        timeout.min(CLEANUP_TIMEOUT),
        context.cancellation,
    )
    .await
    .map_err(|_| ())?;
    if output.stdout.truncated || output.stderr.truncated {
        return Err(());
    }
    Ok(output)
}

fn valid_container_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
