use super::*;

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
/// `[jobs.interpreter]` is protected as a whole sub-table. The
/// interpreter approval's argument: the user typing
/// `--allow interpreter:python3` is approving *the python3 of their
/// toolchain*, but an interpreter's staging inputs — which binary answers
/// the name, which support trees become readable — decide what actually
/// executes under it. A repository-controlled config that answers
/// "python3" with a binary of its own choosing turns the user's typed
/// approval into approval of bytes the user never saw — the `ai.base_url`
/// class of decision: the flag the user types is redirected by an untrusted
/// file. And an interpreter the trusted layers never declared is still an
/// attacker-chosen program, the endpoint-name precedent applied to
/// programs. The whole sub-table is snapshotted and reverted, reported by
/// its dotted name, so the user is told what was ignored. A compiled
/// program's behaviour is fixed and reviewable before the run, which is why
/// `[jobs.runner]` — its sibling — stays an ordinary setting: an interpreter
/// approval voids a security contract, and it would be void against bytes
/// the staging config chose.
pub(crate) struct ProtectedSettings {
    ai_base_url: Option<String>,
    ai_api_key: Option<SecretRef>,
    ai_allow_data_sharing: Option<bool>,
    ai_investigation_max_logical_answering_requests: Option<toml::Value>,
    ai_investigation_max_requested_tool_calls: Option<toml::Value>,
    ai_investigation_max_elapsed_seconds: Option<toml::Value>,
    ai_investigation_max_known_reported_tokens: Option<toml::Value>,
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
    /// The whole `[jobs.interpreter]` sub-table as the trusted layers
    /// declared it: which interpreters may ever resolve, and by extension
    /// which bytes answer them.
    jobs_interpreter: Option<crate::model::InterpreterJobsFile>,
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
        ai_base_url: file.ai.base_url.clone(),
        ai_api_key: file.ai.api_key.clone(),
        ai_allow_data_sharing: file.ai.allow_data_sharing,
        ai_investigation_max_logical_answering_requests: file
            .ai
            .investigation_max_logical_answering_requests
            .clone(),
        ai_investigation_max_requested_tool_calls: file
            .ai
            .investigation_max_requested_tool_calls
            .clone(),
        ai_investigation_max_elapsed_seconds: file.ai.investigation_max_elapsed_seconds.clone(),
        ai_investigation_max_known_reported_tokens: file
            .ai
            .investigation_max_known_reported_tokens
            .clone(),
        run_read_only: file.run.read_only,
        instructions_project: file.instructions.project,
        instructions_personal: file.instructions.personal,
        instructions_project_max_bytes: file.instructions.project_max_bytes,
        instructions_personal_max_bytes: file.instructions.personal_max_bytes,
        instructions_total_max_bytes: file.instructions.total_max_bytes,
        jobs_interpreter: file.jobs.interpreter.clone(),
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
    revert_ai_budget(
        &mut file.ai.investigation_max_logical_answering_requests,
        &before.ai_investigation_max_logical_answering_requests,
        "investigation_max_logical_answering_requests",
        &mut ignored,
    );
    revert_ai_budget(
        &mut file.ai.investigation_max_requested_tool_calls,
        &before.ai_investigation_max_requested_tool_calls,
        "investigation_max_requested_tool_calls",
        &mut ignored,
    );
    revert_ai_budget(
        &mut file.ai.investigation_max_elapsed_seconds,
        &before.ai_investigation_max_elapsed_seconds,
        "investigation_max_elapsed_seconds",
        &mut ignored,
    );
    revert_ai_budget(
        &mut file.ai.investigation_max_known_reported_tokens,
        &before.ai_investigation_max_known_reported_tokens,
        "investigation_max_known_reported_tokens",
        &mut ignored,
    );
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
    // The interpreter universe is protected as a whole sub-table: which
    // interpreters may ever resolve, and by extension which bytes answer
    // them, is trusted-layer business. Reported by its dotted name so the
    // user is told what was ignored.
    if file.jobs.interpreter != before.jobs_interpreter {
        file.jobs.interpreter = before.jobs_interpreter.clone();
        ignored.push("jobs.interpreter".into());
    }
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
