use super::{CodingApplication, CodingError, doctor_probe::ProbeOutcome, host::private_directory};
use saya_agent::CancellationToken;
use saya_config::ResolvedCoding;
use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
use serde_json::{Value, json};

#[cfg(test)]
#[path = "doctor_tests.rs"]
mod doctor_tests;
impl CodingApplication {
    pub async fn doctor(&self, probe_provider: bool) -> Result<Value, CodingError> {
        // A Git that fails its version check is a blocked readiness step, not
        // an error: `coding setup` must still be able to say what to fix.
        let git = self
            .service
            .git_version(&CancellationToken::new())
            .await
            .ok();
        let coding = &self.runtime.resolved.coding;
        let configured_programs = coding
            .check_profiles
            .values()
            .all(|profile| self.program_dir.join(profile.program()).is_file());
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
        let extensions_proven = !extension_health.is_empty()
            && extension_health.iter().all(|item| item["healthy"] == true);
        // Remove only this unique probe directory (created above as doctor-<pid>-<n>);
        // the sandbox battery leaves canary entries in it. Never a user checkout.
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
        let ready = coding.enabled
            && git.is_some()
            && confined
            && writable
            && configured_programs
            && extensions_installed
            && required_protocols
            && extensions_proven
            && native_tool_loop == Some(true);
        let mut blocked: Vec<String> = Vec::new();
        if git.is_none() {
            blocked.push("Install a supported Git: the Git version check failed".to_owned());
        }
        if !coding.enabled {
            blocked.push("Enable coding.enabled in trusted user configuration".to_owned());
        }
        if !confined {
            blocked.push(
                "Host command confinement was not proven; coding commands remain unavailable"
                    .to_owned(),
            );
        }
        if !configured_programs {
            blocked.push(
                "Install configured check programs in the selected trusted program directory"
                    .to_owned(),
            );
        }
        if !required_protocols {
            blocked.push(
                "Configure installed MCP and LSP profiles for the selected coding workflow"
                    .to_owned(),
            );
        }
        if !extensions_proven {
            blocked.push(
                "Run coding doctor --probe to verify configured extension handshakes and shutdown"
                    .to_owned(),
            );
        }
        if !extensions_installed {
            blocked.push(
                "Install configured extension programs; no automatic installation is performed"
                    .to_owned(),
            );
        }
        if let Some(entry) = tool_loop_blocked(model_loop, configured_max_output_tokens) {
            blocked.push(entry);
        }
        let facts = DoctorFacts {
            provider_bindings: self.provider_bindings()?,
            coding: coding.clone(),
            ready,
            git,
            private_state_writable: writable,
            confinement_proven: confined,
            native_provider_tool_loop: native_tool_loop,
            check_programs_installed: configured_programs,
            extensions,
            extension_protocol_runtime_health: extension_health,
            blocked,
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
    pub(super) coding: ResolvedCoding,
    pub(super) ready: bool,
    pub(super) git: Option<String>,
    pub(super) private_state_writable: bool,
    pub(super) confinement_proven: bool,
    pub(super) native_provider_tool_loop: Option<bool>,
    pub(super) check_programs_installed: bool,
    pub(super) extensions: Vec<Value>,
    pub(super) extension_protocol_runtime_health: Vec<Value>,
    pub(super) blocked: Vec<String>,
}

/// Assembles the display-safe report. `token_admission` keeps its published
/// object; `token_ceilings` is additive.
pub(super) fn report(facts: DoctorFacts) -> Value {
    let coding = &facts.coding;
    json!({
        "provider_bindings": facts.provider_bindings,
        "ready": facts.ready,
        "enabled": coding.enabled,
        "limits": {
            "max_children": coding.max_children,
            "max_concurrent_model_requests": coding.max_concurrent_model_requests,
            "max_concurrent_commands": coding.max_concurrent_commands,
            "max_logical_model_requests": coding.max_logical_model_requests,
            "max_physical_transport_attempts": coding.max_physical_transport_attempts,
            "max_tool_calls": coding.max_tool_calls,
            "max_known_tokens": coding.max_known_tokens,
            "max_elapsed_seconds": coding.max_elapsed_seconds,
        },
        "token_admission": {
            "configured": false,
            "mode": "independent_reported_thresholds_pending_shared_owner",
        },
        "token_ceilings": token_ceiling_facts(coding.max_known_tokens, coding.max_job_tokens),
        "git": facts.git,
        "private_state_writable": facts.private_state_writable,
        "confinement_proven": facts.confinement_proven,
        "native_provider_tool_loop": facts.native_provider_tool_loop,
        "check_programs_installed": facts.check_programs_installed,
        "extensions": facts.extensions,
        "extension_protocol_runtime_health": facts.extension_protocol_runtime_health,
        "database_required": false,
        "blocked": facts.blocked,
    })
}

/// The token-ceiling disclosure. The per-loop limit is enforced by every
/// model loop against provider-reported usage; the job-wide total is
/// disclosed as not enforced until the enforcement packet.
pub(super) fn token_ceiling_facts(max_known_tokens: u64, max_job_tokens: Option<u64>) -> Value {
    json!({
        "per_loop_reported_tokens": {
            "limit": max_known_tokens,
            "enforced": true,
            "basis": "provider_reported_usage",
            "note": "each model loop stops once its own reported tokens reach the limit; one in-flight request can overshoot",
        },
        "job_total": {
            "limit": max_job_tokens,
            "enforced": false, // until the enforcement packet
            "default": "not enforced",
            "note": "not enforced unless coding.max_job_tokens is set",
        },
    })
}

/// Additive doctor facts for the tool-loop probe. The boolean
/// `native_provider_tool_loop` keeps its meaning for existing readers; the
/// failure field and the blocked entry name the output-token limit only when
/// the probe ran and the limit cut it off — never provider detail.
pub(super) fn tool_loop_failure_field(
    probe: Option<ProbeOutcome>,
    configured_max_output_tokens: u32,
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
    configured_max_output_tokens: u32,
) -> Option<String> {
    if probe.is_some_and(|outcome| outcome.completed) {
        return None;
    }
    let truncated = probe.is_some_and(|outcome| outcome.truncated_by_output_limit);
    Some(if truncated {
        format!(
            "The configured model hit its output-token limit (ai.max_output_tokens = \
             {configured_max_output_tokens}) before completing the native tool loop: raise \
             ai.max_output_tokens in trusted configuration, then run coding doctor --probe again"
        )
    } else {
        "Run coding doctor --probe and verify the configured provider completes the native tool loop"
            .to_owned()
    })
}

fn add_tool_loop_failure(mut report: Value, failure: Option<Value>) -> Value {
    if let Some(failure) = failure {
        report["native_provider_tool_loop_failure"] = failure;
    }
    report
}
