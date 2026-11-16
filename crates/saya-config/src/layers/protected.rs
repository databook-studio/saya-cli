use super::*;

#[path = "protected/agents.rs"]
mod agents;
#[path = "protected/job.rs"]
mod job;
#[path = "protected/limits.rs"]
mod limits;

/// The values of security-critical settings captured after the user layer
/// merges. Project layers may not change them: a repository's
/// `.saya/config.toml` is untrusted input, and these settings decide
/// where the API key is sent, whether rows leave the machine, and whether
/// engine-level read-only enforcement stays on.
///
/// `ai.show_thinking` is deliberately not on this list. It renders locally, to
/// the person who already sees the answer, and cannot exfiltrate anything the
/// answer does not already show — so it is an ordinary setting the project
/// layer may set without `--trust-project-config`. Adding a setting here is
/// a deliberate decision, not an oversight; this is that decision recorded
/// next to the list it joins.
///
/// `[[ai.endpoints]]` is protected per entry, not as a whole: the set of
/// *names* itself (the project layer may not add an endpoint the trusted
/// layers did not declare — an unnamed endpoint is still an attacker-chosen
/// destination), and per existing endpoint exactly the two fields that
/// redirect traffic or inject a credential, `base_url` and `api_key`. Those
/// are the whole point of the boundary: `base_url` decides where the request
/// goes and `api_key` decides which credential authenticates it, so either
/// one landing in a repository-controlled config points the user's model
/// traffic — and their key — at an attacker. An endpoint's `provider` and
/// `model` stay ordinary settings, like `[ai] model`: they name which model
/// answers, not where the request goes or what it authenticates with.
///
pub(crate) struct ProtectedSettings {
    job: crate::JobFile,
    /// The `[agents]` table, protected whole like `[job]`: it decides which
    /// helper types a job advertises, what each one may do, which model answers
    /// it, and how many children run at once — a model-writable file must not
    /// be able to widen any of that.
    agents: crate::AgentsFile,
    /// The `[tools]` table, protected whole like `[job]`: it decides what
    /// the agent may reach, and a repository's `.saya/config.toml` is
    /// untrusted input — a model-writable file must not be able to widen the
    /// surface a user narrowed, nor narrow away the tools the user's own work
    /// needs.
    tools: crate::ToolsFile,
    /// The `[hooks]` table, protected whole like `[tools]`: every entry is a
    /// command that runs on the user's host at a lifecycle point. A
    /// repository's `.saya/config.toml` is untrusted input, and a
    /// model-writable file must not be able to add a command the agent then
    /// runs — nor remove the user's own.
    hooks: crate::HooksFile,
    /// The `[web_search]` table, protected whole like `[tools]`: it decides
    /// whether the agent may search at all and which host a query goes to,
    /// and a repository's `.saya/config.toml` is untrusted input.
    web_search: crate::WebSearchFile,
    /// The `[mcp_servers]` table, protected whole like `[tools]`: every entry
    /// names a program that runs on the user's host and every tool it
    /// advertises reaches the model. A repository's `.saya/config.toml` is
    /// untrusted input, and a model-writable file must not be able to add a
    /// server, replace one, or retarget one at a different program.
    mcp_servers: crate::McpServersFile,
    /// The `[permissions]` table, protected whole like `[tools]`: it decides
    /// whether a session may enter `bypass` — the one consent that answers
    /// every per-call question — and a repository's `.saya/config.toml` is
    /// untrusted input, so a model-writable file must not be able to grant
    /// it.
    permissions: crate::PermissionsFile,
    ai_base_url: Option<String>,
    ai_api_key: Option<SecretRef>,
    ai_allow_data_sharing: Option<bool>,
    /// The answering sub-tables of `[limits]`, protected whole — see
    /// [`limits`]. Protection is per sub-table so a future `[limits.run]` can
    /// stay ordinary without this one silently becoming unprotected.
    limits: limits::ProtectedLimits,
    run_read_only: Option<bool>,
    /// The `[instructions]` keys, protected one field at a time: they decide
    /// which instruction files are read and how many bytes of them the
    /// model's context is assembled from, so an untrusted layer must not
    /// disable a source, raise a ceiling, or (there is no path key to abuse)
    /// steer any of it.
    instructions_project: Option<bool>,
    instructions_personal: Option<bool>,
    instructions_project_max_bytes: Option<usize>,
    instructions_personal_max_bytes: Option<usize>,
    instructions_total_max_bytes: Option<usize>,
    /// The declared endpoint names, with each one's protected fields. Keyed
    /// by name so a project-layer entry is classified as either an override
    /// of a known endpoint or an addition.
    endpoints: BTreeMap<String, ProtectedEndpoint>,
}

#[derive(Clone)]
struct ProtectedEndpoint {
    base_url: Option<String>,
    api_key: Option<SecretRef>,
}

pub(crate) fn snapshot_protected(file: &ConfigFile) -> ProtectedSettings {
    ProtectedSettings {
        job: job::snapshot(file),
        agents: agents::snapshot(file),
        tools: file.tools.clone(),
        hooks: file.hooks.clone(),
        web_search: file.web_search.clone(),
        mcp_servers: file.mcp_servers.clone(),
        permissions: file.permissions.clone(),
        ai_base_url: file.ai.base_url.clone(),
        ai_api_key: file.ai.api_key.clone(),
        ai_allow_data_sharing: file.ai.allow_data_sharing,
        limits: limits::snapshot(file),
        run_read_only: file.run.read_only,
        instructions_project: file.instructions.project,
        instructions_personal: file.instructions.personal,
        instructions_project_max_bytes: file.instructions.project_max_bytes,
        instructions_personal_max_bytes: file.instructions.personal_max_bytes,
        instructions_total_max_bytes: file.instructions.total_max_bytes,
        endpoints: file
            .ai
            .endpoints
            .iter()
            .map(|endpoint| {
                (
                    endpoint.name.clone(),
                    ProtectedEndpoint {
                        base_url: endpoint.base_url.clone(),
                        api_key: endpoint.api_key.clone(),
                    },
                )
            })
            .collect(),
    }
}

/// Restores the protected settings to their pre-project values and returns
/// the dotted names the project layer tried (and failed) to override.
pub(crate) fn revert_untrusted(file: &mut ConfigFile, before: &ProtectedSettings) -> Vec<String> {
    let mut ignored = Vec::new();
    if job::revert(file, &before.job) {
        ignored.push("job".into());
    }
    if agents::revert(file, &before.agents) {
        ignored.push("agents".into());
    }
    if file.tools != before.tools {
        file.tools = before.tools.clone();
        ignored.push("tools".into());
    }
    if file.hooks != before.hooks {
        file.hooks = before.hooks.clone();
        ignored.push("hooks".into());
    }
    if file.web_search != before.web_search {
        file.web_search = before.web_search.clone();
        ignored.push("web_search".into());
    }
    if file.mcp_servers != before.mcp_servers {
        file.mcp_servers = before.mcp_servers.clone();
        ignored.push("mcp_servers".into());
    }
    if file.permissions != before.permissions {
        file.permissions = before.permissions.clone();
        ignored.push("permissions".into());
    }
    if file.ai.base_url != before.ai_base_url {
        file.ai.base_url = before.ai_base_url.clone();
        ignored.push("ai.base_url".into());
    }
    if file.ai.api_key != before.ai_api_key {
        file.ai.api_key = before.ai_api_key.clone();
        ignored.push("ai.api_key".into());
    }
    if file.ai.allow_data_sharing != before.ai_allow_data_sharing {
        file.ai.allow_data_sharing = before.ai_allow_data_sharing;
        ignored.push("ai.allow_data_sharing".into());
    }
    ignored.append(&mut limits::revert(file, &before.limits));
    if file.run.read_only != before.run_read_only {
        file.run.read_only = before.run_read_only;
        ignored.push("run.read_only".into());
    }
    // Instruction enablement and limits are protected per field: which
    // instruction files load, and how many bytes of them may reach the
    // model's context, is trusted-layer business. Reported by dotted name so
    // the user is told what was ignored.
    revert_instruction(
        &mut file.instructions.project,
        &before.instructions_project,
        "project",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.personal,
        &before.instructions_personal,
        "personal",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.project_max_bytes,
        &before.instructions_project_max_bytes,
        "project_max_bytes",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.personal_max_bytes,
        &before.instructions_personal_max_bytes,
        "personal_max_bytes",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.total_max_bytes,
        &before.instructions_total_max_bytes,
        "total_max_bytes",
        &mut ignored,
    );
    // Endpoints the untrusted layer added wholesale: a name the trusted
    // layers never declared. Removed entirely and reported by name — a bare
    // `ai.endpoints` would not tell the user what to fix.
    let mut added = Vec::new();
    file.ai.endpoints.retain(|endpoint| {
        let declared = before.endpoints.contains_key(&endpoint.name);
        if !declared {
            added.push(format!("ai.endpoints[{:?}]", endpoint.name));
        }
        declared
    });
    ignored.append(&mut added);
    // An existing endpoint reverts field-wise, so the report names which
    // endpoint was touched and which protected field was retargeted.
    for endpoint in &mut file.ai.endpoints {
        let Some(protected) = before.endpoints.get(&endpoint.name) else {
            continue;
        };
        if endpoint.base_url != protected.base_url {
            endpoint.base_url = protected.base_url.clone();
            ignored.push(format!("ai.endpoints[{:?}].base_url", endpoint.name));
        }
        if endpoint.api_key != protected.api_key {
            endpoint.api_key = protected.api_key.clone();
            ignored.push(format!("ai.endpoints[{:?}].api_key", endpoint.name));
        }
    }
    ignored
}
