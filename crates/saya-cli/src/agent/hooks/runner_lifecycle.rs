//! The two lifecycle events and the permission ask, split from the runner so
//! each file holds one concern. They share its bounds and its shell.

use saya_config::HookEvent;

use super::{HookRun, Hooks, MAX_SESSION_CONTEXT_BYTES, payload};

impl Hooks {
    /// Runs the `SessionStart` hooks once and returns their stdout as the
    /// bounded context offered to the model. A hook that exits non-zero is a
    /// diagnostic; only a clean hook's stdout is context.
    pub(crate) async fn session_start(&self) -> Option<String> {
        let hooks = self.hooks.for_event(HookEvent::SessionStart);
        if hooks.is_empty() {
            return None;
        }
        let payload = payload::lifecycle_event(&self.caller, HookEvent::SessionStart);
        let mut context = String::new();
        for hook in hooks {
            match self.run(hook, &payload).await {
                HookRun::Exited {
                    code: Some(0),
                    stdout,
                    ..
                } => {
                    if !stdout.trim().is_empty() {
                        if !context.is_empty() {
                            context.push('\n');
                        }
                        context.push_str(stdout.trim());
                    }
                }
                other => self.diagnostic(HookEvent::SessionStart, other),
            }
        }
        if context.is_empty() {
            return None;
        }
        Some(saya_agent::bound_tool_transcript(
            &context,
            MAX_SESSION_CONTEXT_BYTES,
        ))
    }

    /// Runs the `SessionEnd` hooks: best-effort, after the session's own
    /// cancellation may already have fired. They observe and change nothing,
    /// and each is still bounded by its own timeout.
    pub(crate) async fn session_end(&self) {
        let hooks = self.hooks.for_event(HookEvent::SessionEnd);
        if hooks.is_empty() {
            return;
        }
        let payload = payload::lifecycle_event(&self.caller, HookEvent::SessionEnd);
        for hook in hooks {
            let run = self.run_uncancelled(hook, &payload).await;
            if !matches!(run, HookRun::Exited { code: Some(0), .. }) {
                self.diagnostic(HookEvent::SessionEnd, run);
            }
        }
    }

    /// Runs this call's `PermissionRequest` hooks, before the operator would
    /// be asked. `Err` carries the blocking hook's bounded, redacted stderr;
    /// exit `0` is not an approval — the caller still asks the operator.
    pub(crate) async fn permission_request(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
    ) -> Result<(), String> {
        let mut input: Option<String> = None;
        for hook in self.watching(HookEvent::PermissionRequest, tool) {
            let payload = input
                .get_or_insert_with(|| payload::permission_event(&self.caller, tool, arguments));
            match self.run(hook, payload).await {
                HookRun::Exited {
                    code: Some(2),
                    stderr,
                    ..
                } => {
                    return Err(if stderr.is_empty() {
                        "a permission_request hook denied this call".to_owned()
                    } else {
                        stderr
                    });
                }
                HookRun::Exited { code: Some(0), .. } => {}
                other => self.diagnostic(HookEvent::PermissionRequest, other),
            }
        }
        Ok(())
    }
}
