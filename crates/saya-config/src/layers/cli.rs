use super::*;

/// The flag each `[limits.job]` dimension is stated by. `tool_calls` and
/// `known_tokens` have no flag, so they are only ever reached from a config
/// file; the fallback names a flag that does not exist rather than panicking,
/// and is never taken.
fn job_limit_flag(field: &str) -> String {
    match field {
        "logical_requests" => "--coding-max-logical-model-requests".into(),
        "transport_attempts" => "--coding-max-physical-transport-attempts".into(),
        "job_tokens" => "--coding-max-job-tokens".into(),
        "elapsed_seconds" => "--coding-max-elapsed-seconds".into(),
        _ => format!("--coding-{}", field.replace('_', "-")),
    }
}

pub(crate) fn apply_cli(file: &mut ConfigFile, cli: &CliOverrides, recorder: &mut Recorder) {
    // The `--coding-max-*` flags state `[limits.job]` keys, so each one is
    // attributed to the key it overrides rather than to the section.
    crate::limits::merge_job(&mut file.limits.job, &cli.job, recorder, |field| {
        SettingSource::Cli {
            flag: job_limit_flag(field),
        }
    });
    crate::agents::merge(&mut file.agents, &cli.agents, recorder, |field| {
        SettingSource::Cli {
            flag: format!("--agents-{}", field.replace('_', "-")),
        }
    });
    crate::tools::merge(&mut file.tools, &cli.tools, recorder, |_| {
        SettingSource::Cli {
            flag: "--tools".into(),
        }
    });
    // Flag rules add to the trusted config rules: they are one more statement
    // from the same operator, so a flag never removes what the user's own
    // file stated.
    if cli
        .permissions
        .allow
        .as_deref()
        .is_some_and(|rules| !rules.is_empty())
    {
        let base = file.permissions.allow.get_or_insert_with(Vec::new);
        base.extend(cli.permissions.allow.iter().flatten().cloned());
        recorder.record(
            "permissions.allow",
            SettingSource::Cli {
                flag: "--allow-command".into(),
            },
        );
    }
    if cli
        .permissions
        .deny
        .as_deref()
        .is_some_and(|rules| !rules.is_empty())
    {
        let base = file.permissions.deny.get_or_insert_with(Vec::new);
        base.extend(cli.permissions.deny.iter().flatten().cloned());
        recorder.record(
            "permissions.deny",
            SettingSource::Cli {
                flag: "--deny-command".into(),
            },
        );
    }
    if cli.provider.is_some() {
        file.ai.provider = cli.provider;
        recorder.record(
            "ai.provider",
            SettingSource::Cli {
                flag: "--provider".into(),
            },
        );
    }
    if cli.model.is_some() {
        file.ai.model = cli.model.clone();
        recorder.record(
            "ai.model",
            SettingSource::Cli {
                flag: "--model".into(),
            },
        );
    }
    if cli.allow_data_sharing.is_some() {
        file.ai.allow_data_sharing = cli.allow_data_sharing;
        recorder.record(
            "ai.allow_data_sharing",
            SettingSource::Cli {
                flag: "--allow-data-sharing".into(),
            },
        );
    }
    if cli.max_rows.is_some() {
        file.run.max_rows = cli.max_rows;
        recorder.record(
            "run.max_rows",
            SettingSource::Cli {
                flag: "--max-rows".into(),
            },
        );
    }
    if cli.candidates.is_some() {
        file.run.candidates = cli.candidates;
        recorder.record(
            "run.candidates",
            SettingSource::Cli {
                flag: "--candidates".into(),
            },
        );
    }
    if cli.show_thinking.is_some() {
        file.ai.show_thinking = cli.show_thinking;
        recorder.record(
            "ai.show_thinking",
            SettingSource::Cli {
                flag: "--show-thinking".into(),
            },
        );
    }
    if cli.theme.is_some() {
        file.ui.theme = cli.theme;
        recorder.record(
            "ui.theme",
            SettingSource::Cli {
                flag: "--theme".into(),
            },
        );
    }
    if cli.color.is_some() {
        file.output.color = cli.color;
        recorder.record(
            "output.color",
            SettingSource::Cli {
                flag: "--no-color".into(),
            },
        );
    }
}
