use super::*;

pub(crate) fn apply_env(
    file: &mut ConfigFile,
    env: &BTreeMap<String, String>,
    recorder: &mut Recorder,
    source: &dyn Fn(&str) -> SettingSource,
) -> Result<(), ConfigError> {
    apply_string(
        &mut file.ai.model,
        env,
        "SAYA_AI_MODEL",
        "ai.model",
        recorder,
        source,
    );
    apply_string(
        &mut file.ai.base_url,
        env,
        "SAYA_AI_BASE_URL",
        "ai.base_url",
        recorder,
        source,
    );
    apply_secret(
        &mut file.ai.api_key,
        env,
        "SAYA_AI_API_KEY",
        "ai.api_key",
        recorder,
        source,
    );
    apply_parsed(
        &mut file.ai.provider,
        env,
        "SAYA_AI_PROVIDER",
        "ai.provider",
        recorder,
        source,
        AiProvider::parse,
    )?;
    apply_string(
        &mut file.ai.model,
        env,
        "SAYA_MODEL",
        "ai.model",
        recorder,
        source,
    );
    apply_string(
        &mut file.ai.base_url,
        env,
        "SAYA_PROVIDER_BASE_URL",
        "ai.base_url",
        recorder,
        source,
    );
    apply_secret(
        &mut file.ai.api_key,
        env,
        "SAYA_API_KEY",
        "ai.api_key",
        recorder,
        source,
    );
    apply_parsed(
        &mut file.ai.provider,
        env,
        "SAYA_PROVIDER",
        "ai.provider",
        recorder,
        source,
        AiProvider::parse,
    )?;
    apply_parsed(
        &mut file.ai.allow_data_sharing,
        env,
        "SAYA_ALLOW_DATA_SHARING",
        "ai.allow_data_sharing",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.ai.compaction,
        env,
        "SAYA_COMPACTION",
        "ai.compaction",
        recorder,
        source,
        CompactionMode::parse,
    )?;
    apply_parsed(
        &mut file.run.read_only,
        env,
        "SAYA_READ_ONLY",
        "run.read_only",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.max_rows,
        env,
        "SAYA_MAX_ROWS",
        "run.max_rows",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.max_iterations,
        env,
        "SAYA_MAX_ITERATIONS",
        "run.max_iterations",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.candidates,
        env,
        "SAYA_CANDIDATES",
        "run.candidates",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.query_timeout_seconds,
        env,
        "SAYA_QUERY_TIMEOUT_SECONDS",
        "run.query_timeout_seconds",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.output.format,
        env,
        "SAYA_OUTPUT_FORMAT",
        "output.format",
        recorder,
        source,
        OutputFormat::parse,
    )?;
    Ok(())
}
