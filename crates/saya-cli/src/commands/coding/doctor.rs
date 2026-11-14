use super::{
    CodingApplication, CodingError, doctor_budget, doctor_probe::ProbeOutcome,
    host::private_directory,
};
use saya_agent::CancellationToken;
use saya_config::ResolvedJobLimits;
use saya_harness::coding::git::GitCompatibility;
use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
use serde_json::{Value, json};

#[cfg(all(test, unix))]
#[path = "doctor_tests.rs"]
mod doctor_tests;
impl CodingApplication {
    pub async fn doctor(&self, probe_provider: bool) -> Result<Value, CodingError> {
        // A Git that fails its version check is a blocked readiness step, not
        // an error: `coding setup` must still be able to say what to fix.
        let git_probe = self
            .service
            .git_version_info(&CancellationToken::new())
            .await;
        let git_info = git_probe.as_ref().ok();
        let git = git_info.as_ref().map(|info| info.raw().to_owned());
        let git_compatibility = git_info.as_ref().map(|info| {
            let compatibility = info.compatibility();
            json!({
                "version": compatibility.version().to_string(),
                "version_known": compatibility.version_known(),
                "version_source": if compatibility.version_known() { "reported" } else { "assumed_minimum" },
                "minimum_supported": GitCompatibility::minimum_supported_version().to_string(),
                "lazy_fetch_protection": compatibility.lazy_fetch_protection().as_str(),
                "worktree_list_zero_terminated": compatibility.worktree_list_zero_terminated(),
            })
        });
        let coding = &self.runtime.resolved.coding;
        let agents = &self.runtime.resolved.agents;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let path = self.state_root.join(format!(
            "doctor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let probe = private_directory(&path)?;
        let confinement = RunSandbox::new([probe.clone()], std::iter::empty())
            .and_then(|sandbox| sandbox.prepare(&self.program_dir));
        let confined = matches!(&confinement, Ok(SandboxProvision::Proven { .. }));
        let extension_health = match &confinement {
            Ok(SandboxProvision::Proven { spawn, .. }) if probe_provider => {
                self.probe_extensions(spawn).await
            }
            _ => Vec::new(),
        };
        let extensions_configured = !coding.extensions.is_empty();
        let extension_failed = extension_health.iter().any(|item| item["healthy"] != true);
        // Remove only this unique probe directory (created above as doctor-<pid>-<n>);
        // this is Saya-owned probe state, never a user checkout.
        let writable = std::fs::remove_dir_all(&probe).is_ok();
        let extensions=coding.extensions.iter().map(|(name,profile)|json!({"name":name,"protocol":format!("{:?}",profile.protocol()),"installed":self.program_dir.join(profile.program()).is_file(),"tool_policy_count":profile.tools().len()})).collect::<Vec<_>>();
        let extensions_installed = extensions.iter().all(|item| item["installed"] == true);
        let required_protocols = coding
            .extensions
            .values()
            .any(|p| p.protocol() == saya_config::ExtensionProtocol::Mcp)
            && coding
                .extensions
                .values()
                .any(|p| p.protocol() == saya_config::ExtensionProtocol::Lsp);
        let model_loop = if probe_provider {
            Some(self.probe_provider().await?)
        } else {
            None
        };
        let native_tool_loop = model_loop.map(|outcome| outcome.completed);
        let configured_max_output_tokens = self.runtime.resolved.ai.max_output_tokens;
        // `blocked` is a required capability that is missing; `optional` is
        // something this install simply has not configured. Only a blocked
        // item makes the install unready, so a fresh install with a configured
        // provider reports ready.
        let mut blocked = Vec::new();
        let mut optional = Vec::new();
        if git.is_none() {
            let detail = git_probe.as_ref().err().map_or_else(
                || "the Git version check failed".to_owned(),
                std::string::ToString::to_string,
            );
            blocked.push(format!("Install a supported Git: {detail}"));
        }
        // Host command confinement is a macOS capability: the Linux probe
        // fails closed by design, so an unproven probe there is not a missing
        // required capability. On macOS the promise is confinement, and a
        // refusal blocks the install.
        if !confined {
            if cfg!(target_os = "macos") {
                blocked.push(
                    "Host command confinement was not proven; job commands remain unavailable"
                        .to_owned(),
                );
            } else {
                optional.push(
                    "Host command confinement is not available on this platform; job commands \
                     run in the host lane"
                        .to_owned(),
                );
            }
        }
        if !writable {
            blocked.push(
                "Make the private state directory writable: job state cannot be stored".to_owned(),
            );
        }
        if extensions_configured && !extensions_installed {
            blocked.push(
                "Install configured extension programs; no automatic installation is performed"
                    .to_owned(),
            );
        }
        if extension_failed {
            blocked.push(
                "A configured extension failed its handshake or shutdown; fix it and run \
                 saya job doctor --probe again"
                    .to_owned(),
            );
        }
        if extensions_configured && extension_health.is_empty() {
            optional.push(
                "Run saya job doctor --probe to verify configured extension handshakes and shutdown"
                    .to_owned(),
            );
        }
        if !required_protocols {
            optional
                .push("Configure installed MCP and LSP profiles for the selected job".to_owned());
        }
        match model_loop {
            None => optional.push(
                "Run saya job doctor --probe and verify the configured provider completes the \
                 native tool loop"
                    .to_owned(),
            ),
            Some(_) => {
                if let Some(entry) = tool_loop_blocked(model_loop, configured_max_output_tokens) {
                    blocked.push(entry);
                }
            }
        }
        let ready = blocked.is_empty();
        let in_place = self.local_root().map(|local| {
            json!({
                "path": local.path().to_string_lossy(),
                "root_id": local.id().as_str(),
            })
        });
        let facts = DoctorFacts {
            provider_bindings: self.provider_bindings()?,
            job_limits: *super::limits::resolved(&self.runtime.resolved),
            agents: agents.clone(),
            ready,
            git,
            git_compatibility,
            private_state_writable: writable,
            confinement_proven: confined,
            native_provider_tool_loop: native_tool_loop,
            extensions,
            extension_protocol_runtime_health: extension_health,
            in_place,
            blocked,
            optional,
        };
        Ok(add_tool_loop_failure(
            report(facts),
            tool_loop_failure_field(model_loop, configured_max_output_tokens),
        ))
    }
}

/// The facts `doctor` has already gathered. The report shape itself is a pure
/// function of them, so the published JSON contract is provable without a
/// provider, sandbox, or Git probe.
pub(super) struct DoctorFacts {
    pub(super) provider_bindings: Value,
    /// The `[limits.job]` ceilings and mode. The `[job]` settings the rest
    /// of this report reads — the extensions and the program directory —
    /// arrive already gathered.
    pub(super) job_limits: ResolvedJobLimits,
    pub(super) agents: saya_config::ResolvedAgents,
    pub(super) ready: bool,
    pub(super) git: Option<String>,
    /// The detected version and the compatibility paths it selects, or `None`
    /// when the version probe failed.
    pub(super) git_compatibility: Option<Value>,
    pub(super) private_state_writable: bool,
    pub(super) confinement_proven: bool,
    pub(super) native_provider_tool_loop: Option<bool>,
    pub(super) extensions: Vec<Value>,
    pub(super) extension_protocol_runtime_health: Vec<Value>,
    /// The in-place root's display identity, or `None` for a Git root: an
    /// in-place job has no managed worktree to report.
    pub(super) in_place: Option<Value>,
    pub(super) blocked: Vec<String>,
    /// Not configured, and not required: an install without these is ready.
    pub(super) optional: Vec<String>,
}

/// Assembles the display-safe report. `token_admission` keeps its published
/// object; `token_ceilings` is additive.
pub(super) fn report(facts: DoctorFacts) -> Value {
    let job_limits = &facts.job_limits;
    let agents = &facts.agents;
    let in_place = facts.in_place.clone();
    let mut report = json!({
        "provider_bindings": facts.provider_bindings,
        "ready": facts.ready,
        "budget_mode": job_limits.budget_mode,
        "limits": doctor_budget::limits(job_limits, agents),
        "operational_bounds": doctor_budget::operational_bounds(),
        "token_admission": {
            "configured": false,
            "mode": "independent_reported_thresholds_pending_shared_owner",
        },
        "token_ceilings": doctor_budget::token_ceilings(job_limits),
        "git": facts.git,
        "git_compatibility": facts.git_compatibility,
        "private_state_writable": facts.private_state_writable,
        "confinement_proven": facts.confinement_proven,
        "native_provider_tool_loop": facts.native_provider_tool_loop,
        "extensions": facts.extensions,
        "extension_protocol_runtime_health": facts.extension_protocol_runtime_health,
        "database_required": false,
        "blocked": facts.blocked,
        "optional": facts.optional,
    });
    // An in-place root has no managed worktree: the report states the folder
    // and its scratch instead, and a Git report keeps its exact key set.
    if let Some(in_place) = in_place {
        report["mode"] = json!("in_place");
        report["worktree"] = Value::Null;
        report["root"] = json!({
            "path": in_place["path"],
            "root_id": in_place["root_id"],
            "git": false,
        });
        report["scratch"] = json!(".saya/inplace");
    }
    report
}

/// Additive doctor facts for the tool-loop probe. The boolean
/// `native_provider_tool_loop` keeps its meaning for existing readers; the
/// failure field and the blocked entry name the output-token limit only when
/// the probe ran and the limit cut it off — never provider detail.
pub(super) fn tool_loop_failure_field(
    probe: Option<ProbeOutcome>,
    configured_max_output_tokens: Option<u32>,
) -> Option<Value> {
    probe
        .is_some_and(|outcome| outcome.truncated_by_output_limit)
        .then(|| {
            json!({
                "reason": "output_limit",
                "configured_max_output_tokens": configured_max_output_tokens,
                "action": "raise ai.max_output_tokens in trusted configuration",
            })
        })
}

pub(super) fn tool_loop_blocked(
    probe: Option<ProbeOutcome>,
    configured_max_output_tokens: Option<u32>,
) -> Option<String> {
    if probe.is_some_and(|outcome| outcome.completed) {
        return None;
    }
    let truncated = probe.is_some_and(|outcome| outcome.truncated_by_output_limit);
    Some(if truncated {
        let ceiling = configured_max_output_tokens
            .map(|configured| format!(" (ai.max_output_tokens = {configured})"))
            .unwrap_or_default();
        format!(
            "The configured model hit its output-token limit{ceiling} before completing the \
             native tool loop: raise ai.max_output_tokens in trusted configuration, then run \
             saya job doctor --probe again"
        )
    } else {
        "Run saya job doctor --probe and verify the configured provider completes the native tool loop"
            .to_owned()
    })
}

fn add_tool_loop_failure(mut report: Value, failure: Option<Value>) -> Value {
    if let Some(failure) = failure {
        report["native_provider_tool_loop_failure"] = failure;
    }
    report
}
