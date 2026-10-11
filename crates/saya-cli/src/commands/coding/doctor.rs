use super::{CodingApplication, CodingError, host::private_directory};
use saya_agent::CancellationToken;
use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
use serde_json::{Value, json};
impl CodingApplication {
    pub async fn doctor(&self, probe_provider: bool) -> Result<Value, CodingError> {
        let git = self
            .service
            .git_version(&CancellationToken::new())
            .await
            .map_err(CodingError::owner)?;
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
        let ready = coding.enabled
            && confined
            && writable
            && configured_programs
            && extensions_installed
            && required_protocols
            && extensions_proven
            && model_loop == Some(true);
        let mut blocked = Vec::new();
        if !coding.enabled {
            blocked.push("Enable coding.enabled in trusted user configuration");
        }
        if !confined {
            blocked.push(
                "Host command confinement was not proven; coding commands remain unavailable",
            );
        }
        if !configured_programs {
            blocked.push(
                "Install configured check programs in the selected trusted program directory",
            );
        }
        if !required_protocols {
            blocked
                .push("Configure installed MCP and LSP profiles for the selected coding workflow");
        }
        if !extensions_proven {
            blocked.push(
                "Run coding doctor --probe to verify configured extension handshakes and shutdown",
            );
        }
        if !extensions_installed {
            blocked.push(
                "Install configured extension programs; no automatic installation is performed",
            );
        }
        if model_loop != Some(true) {
            blocked.push("Run coding doctor --probe and verify the configured provider completes the native tool loop");
        }
        Ok(
            json!({"provider_bindings":self.provider_bindings()?,"ready":ready,"enabled":coding.enabled,"limits":{"max_children":coding.max_children,"max_concurrent_model_requests":coding.max_concurrent_model_requests,"max_concurrent_commands":coding.max_concurrent_commands,"max_logical_model_requests":coding.max_logical_model_requests,"max_physical_transport_attempts":coding.max_physical_transport_attempts,"max_tool_calls":coding.max_tool_calls,"max_known_tokens":coding.max_known_tokens,"max_elapsed_seconds":coding.max_elapsed_seconds},"token_admission":{"configured":false,"mode":"independent_reported_thresholds_pending_shared_owner"},"git":git,"private_state_writable":writable,"confinement_proven":confined,"native_provider_tool_loop":model_loop,"check_programs_installed":configured_programs,"extensions":extensions,"extension_protocol_runtime_health":extension_health,"database_required":false,"blocked":blocked}),
        )
    }
}
