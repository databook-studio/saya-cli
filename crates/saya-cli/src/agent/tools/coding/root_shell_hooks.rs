//! The root shell's test hooks: construction that proves the sandbox the way
//! [`build`](super::build) does, and construction over the host lane alone,
//! as a host with no proven sandbox builds one. Both go through the same
//! [`advertised`](super::shell_policy::advertised) gate, so neither can drift
//! from what the root really does.

use std::path::Path;

use saya_harness::runner::HostPolicy;

use super::RootShell;
use super::shell_policy::{
    Background, ShellPlacement, ShellTool, advertised, compose_host_only, compose_sandboxed, prove,
};
use crate::agent::tools::consent::Prompt;

/// Builds a root shell whose composition is deferred to a blocking task
/// started here. The test seam for the deferral: `compose` runs exactly once,
/// at build, and the first call waits for it.
pub(crate) fn build_deferred_with<F>(compose: F) -> Option<RootShell>
where
    F: FnOnce() -> Option<ShellTool> + Send + 'static,
{
    Some(RootShell::deferred(compose))
}

/// Builds over a caller-supplied prompt, proving the sandbox the way
/// [`build`](super::build) does. `None` where the sandbox does not prove on
/// this host.
pub(crate) fn build_with_policy(
    checkout_root: &Path,
    policy: saya_agent::ApprovalPolicy,
    prompt: Option<Prompt>,
    cancellation: saya_agent::CancellationToken,
) -> Option<RootShell> {
    if !advertised(policy) {
        return None;
    }
    let spawn = prove(checkout_root, policy)?;
    Some(RootShell::new(compose_sandboxed(
        ShellPlacement::root(checkout_root),
        policy,
        spawn,
        prompt,
        HostPolicy::default(),
        cancellation,
        // The fixture builds the root's shell, which owns its background runs.
        Background::Owned,
        saya_config::CommandRules::default(),
    )))
}

/// Builds over the host lane alone, as a host with no proven sandbox does,
/// with the shell's full captured output saved under `spool`.
pub(crate) fn build_host_only_with_spool(
    checkout_root: &Path,
    policy: saya_agent::ApprovalPolicy,
    prompt: Option<Prompt>,
    host: HostPolicy,
    cancellation: saya_agent::CancellationToken,
    spool: Option<&Path>,
) -> Option<RootShell> {
    if !advertised(policy) {
        return None;
    }
    let tool = compose_host_only(
        checkout_root,
        policy,
        prompt,
        host,
        cancellation,
        Background::Owned,
        spool,
        saya_config::CommandRules::default(),
    )?;
    Some(RootShell::new(tool))
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::{ApprovalPolicy, CancellationToken, ToolError};
    use std::sync::mpsc;
    use std::time::Duration;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "saya-root-shell-defer-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn deferred_composition_starts_at_build_and_blocks_the_first_call() {
        let root = temp_root("started");
        let compose_root = root.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let shell = build_deferred_with(move || {
            started_tx.send(()).expect("the test is listening");
            release_rx
                .recv()
                .expect("the test releases the composition");
            compose_host_only(
                &compose_root,
                ApprovalPolicy::Bypass,
                None,
                HostPolicy::default(),
                CancellationToken::new(),
                Background::Owned,
                None,
                saya_config::CommandRules::default(),
            )
        })
        .expect("the deferred shell is built");

        // The probe is dispatched at build, before any call asks for it.
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("composition starts at build");

        let call = tokio::spawn(async move {
            shell
                .call(
                    "shell",
                    &serde_json::json!({"command": "printf deferred-ok"}),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !call.is_finished(),
            "the first call waits for the composition"
        );
        release_tx.send(()).expect("the composition is waiting");
        let result = tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the call completes")
            .expect("the call task does not panic");
        let value = result.expect("the composed shell runs the command");
        assert!(
            value.to_string().contains("deferred-ok"),
            "the composed shell served the call: {value}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn a_deferred_composition_that_yields_no_shell_refuses_the_call() {
        let shell = build_deferred_with(|| None).expect("the deferred shell is built");
        let error = shell
            .call("shell", &serde_json::json!({"command": "true"}))
            .await
            .expect_err("no composed shell serves the call");
        assert!(
            matches!(error, ToolError::Runner(_)),
            "the refusal names the runner: {error}"
        );
    }
}
