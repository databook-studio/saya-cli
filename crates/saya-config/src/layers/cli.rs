use super::*;

pub(crate) fn apply_cli(file: &mut ConfigFile, cli: &CliOverrides, recorder: &mut Recorder) {
    crate::coding::merge(&mut file.coding, &cli.coding, recorder, |field| {
        SettingSource::Cli {
            flag: match field {
                "enabled" => "--coding".into(),
                _ => format!("--coding-{}", field.replace('_', "-")),
            },
        }
    });
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
