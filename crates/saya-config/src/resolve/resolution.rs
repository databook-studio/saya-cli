use super::*;

pub fn resolve(input: ResolutionInput) -> Result<ResolvedConfig, ConfigError> {
    let mut file = ConfigFile::default();
    let mut recorder = Recorder::default();
    if let Some(user) = input.user.as_ref() {
        require_unique_endpoints(&user.ai.endpoints)?;
        merge(&mut file, user, &mut recorder, SettingSource::UserFile);
    }
    let protected = snapshot_protected(&file);
    let provenance_before_project = recorder.snapshot();
    if let Some(project) = input.project.as_ref() {
        require_unique_endpoints(&project.ai.endpoints)?;
        // A project-layer `[host_commands]` is a hard refusal — not a
        // revert: a model-writable file must never shape unsandboxed
        // execution (not enable it, not widen its timeout, not name its
        // env), and `--trust-project-config` does not unlock it.
        if !project.host_commands.pass_env.is_empty()
            || project.host_commands.timeout_seconds.is_some()
        {
            return Err(ConfigError::HostCommandsFromProject);
        }
        // A project-layer `[session_commands]` is a hard refusal — not a
        // revert: a model-writable deny could name every `[jobs.runner]`
        // allow entry, herding the session's work onto the unsandboxed lane.
        if !project.session_commands.deny.is_empty() {
            return Err(ConfigError::SessionCommandsFromProject);
        }
        merge(
            &mut file,
            project,
            &mut recorder,
            SettingSource::ProjectFile,
        );
    }
    let ignored_project_overrides = if input.cli.trust_project_config {
        Vec::new()
    } else {
        let ignored = revert_untrusted(&mut file, &protected);
        // The boundary reverted those keys to their trusted values, so the
        // recorder rolls them back to what they were attributed as before
        // the project layer merged — the ignored list is the key set.
        recorder.rollback(&ignored, &provenance_before_project);
        ignored
    };
    let env_file_source = |var: &str| SettingSource::EnvFile {
        var: var.to_owned(),
    };
    let process_env_source = |var: &str| SettingSource::Env {
        var: var.to_owned(),
    };
    apply_env(&mut file, &input.env_file, &mut recorder, &env_file_source)?;
    apply_env(
        &mut file,
        &input.process_env,
        &mut recorder,
        &process_env_source,
    )?;
    apply_cli(&mut file, &input.cli, &mut recorder);
    // `profile_name` mirrors the selection chain below: the flag, then the
    // process environment, then `default_profile` (as attributed by the
    // merge), then the single-profile inference as a plain default.
    let profile_source = if input.cli.profile.is_some() {
        Some(SettingSource::Cli {
            flag: "--profile".into(),
        })
    } else if input.process_env.contains_key("SAYA_PROFILE") {
        Some(SettingSource::Env {
            var: "SAYA_PROFILE".into(),
        })
    } else {
        recorder
            .origin_of("default_profile")
            .map(|origin| origin.source.clone())
    };
    let selected = input
        .cli
        .profile
        .or_else(|| input.process_env.get("SAYA_PROFILE").cloned())
        .or(file.default_profile.clone())
        .or_else(|| {
            (input.connections.profiles.len() == 1)
                .then(|| input.connections.profiles.keys().next().cloned())
                .flatten()
        });
    if selected.is_none() && input.connections.profiles.len() > 1 {
        return Err(ConfigError::MissingProfile);
    }
    if let Some(source) = profile_source {
        recorder.record_reference("profile_name", source, None);
    }
    let environment_profile = input.env_file.contains_key("SAYA_DB_TYPE")
        || input.process_env.contains_key("SAYA_DB_TYPE");
    let profile = selected
        .as_ref()
        .map(|name| match input.connections.profiles.get(name).cloned() {
            Some(profile) => Ok(Some(profile)),
            None if environment_profile => Ok(None),
            None => Err(ConfigError::UnknownProfile(name.clone())),
        })
        .transpose()?
        .flatten();
    let profile = overlay_database_environment(profile, &input.env_file, &input.process_env)?;
    let memory = crate::memory::resolve(&file.memory)?;
    let instructions = crate::instructions::resolve(&file.instructions)?;
    let investigation_budgets = crate::investigation_budget::resolve(&file.ai)?;
    let context_byte_budget = file
        .ai
        .context_byte_budget
        .unwrap_or(DEFAULT_CONTEXT_BYTE_BUDGET);
    require_context_byte_budget(context_byte_budget)?;
    require_context_window_tokens(file.ai.context_window_tokens)?;
    let retry_delays_ms = file
        .ai
        .retry_delays_ms
        .clone()
        .unwrap_or_else(|| DEFAULT_RETRY_DELAYS_MS.to_vec());
    require_retry_delays(&retry_delays_ms)?;
    let candidates = file.run.candidates.unwrap_or(DEFAULT_CANDIDATES);
    require_candidates(candidates)?;
    let max_iterations = file.run.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS);
    require_max_iterations(max_iterations)?;
    let jobs = crate::jobs::resolve(&file.jobs)?;
    let host_commands = crate::jobs::resolve_host_commands(file.host_commands.clone())?;
    let session_deny = crate::jobs::resolve_session_deny(file.session_commands.clone())?;
    let ai = ResolvedAi {
        provider: file.ai.provider.unwrap_or(AiProvider::Ollama),
        model: file
            .ai
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.into()),
        // Cloned, not moved: the endpoint map inherits these same fields as
        // its fallback, and the file is read again by `resolve_endpoints`.
        base_url: file.ai.base_url.clone(),
        api_key: file.ai.api_key.clone(),
        allow_data_sharing: file.ai.allow_data_sharing.unwrap_or(false),
        temperature: file.ai.temperature.unwrap_or(0.1),
        timeout_seconds: file.ai.timeout_seconds.unwrap_or(60),
        idle_timeout_seconds: file.ai.idle_timeout_seconds.unwrap_or(90),
        max_output_tokens: file.ai.max_output_tokens.unwrap_or(4096),
        max_output_tokens_is_default: file.ai.max_output_tokens.is_none(),
        context_byte_budget,
        context_window_tokens: file.ai.context_window_tokens,
        show_thinking: file.ai.show_thinking.unwrap_or(false),
        compaction: file.ai.compaction.unwrap_or_default(),
        retry_delays_ms,
    };
    let endpoints = resolve_endpoints(&file.ai, &ai)?;
    let mut provenance = recorder.finish();
    apply_user_facing_defaults(&mut provenance, &endpoints);
    Ok(ResolvedConfig {
        profile_name: selected,
        profile,
        ai,
        investigation_budgets,
        max_rows: file.run.max_rows.unwrap_or(1000),
        read_only: file.run.read_only.unwrap_or(true),
        max_iterations,
        candidates,
        jobs,
        host_commands,
        session_deny,
        query_timeout_seconds: file.run.query_timeout_seconds.unwrap_or(60),
        output_format: file.output.format.unwrap_or(OutputFormat::Text),
        output_color: file.output.color.unwrap_or(ColorChoice::Auto),
        ui_theme: file.ui.theme.unwrap_or(ThemeChoice::Auto),
        memory,
        instructions,
        ignored_project_overrides,
        endpoints,
        provenance,
    })
}
