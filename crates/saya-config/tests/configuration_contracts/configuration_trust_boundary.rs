use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, SettingSource, resolve,
};

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

#[test]
fn untrusted_project_job_table_is_restored_with_all_provenance() {
    let user = ConfigFile::from_toml(
        "[agents]\nmax_children = 1\n\n[job]\nprogram_dir = '/opt/saya-programs'\nautomatic_lsp_profile = 'rust'\n\n[job.extensions.rust]\nprotocol = 'lsp'\nprogram = 'rust-lsp'\nargv = []\nlanguage = 'rust'\n",
    )
    .unwrap();
    let project = ConfigFile::from_toml(
        "[agents]\nmax_children = 3\n\n[job]\nprogram_dir = '/tmp/attacker-programs'\nautomatic_lsp_profile = 'other'\n\n[limits.job]\ntool_calls = 299\njob_tokens = 7\n\n[job.extensions.other]\nprotocol = 'lsp'\nprogram = 'other-lsp'\nargv = []\nlanguage = 'rust'\n",
    )
    .unwrap();
    let resolved = resolve(
        empty()
            .with_user(user.clone())
            .with_project(project.clone()),
    )
    .unwrap();
    assert_eq!(resolved.agents.max_children, 1);
    assert_eq!(
        resolved.coding.program_dir.as_deref(),
        Some(std::path::Path::new("/opt/saya-programs"))
    );
    assert_eq!(resolved.limits.job.tool_calls.value(), Some(10_000));
    assert_eq!(
        resolved.limits.job.job_tokens,
        saya_config::BudgetLimit::Unlimited,
        "the untrusted project ceiling is reverted to the unset default"
    );
    assert_eq!(
        resolved.coding.automatic_lsp_profile.as_deref(),
        Some("rust")
    );
    assert_eq!(
        resolved.ignored_project_overrides,
        ["job", "agents", "limits.job"]
    );
    for key in [
        "job.automatic_lsp_profile",
        "agents.max_children",
        "agents.max_concurrent_model_requests",
        "job.program_dir",
        "limits.job.logical_requests",
        "limits.job.transport_attempts",
        "limits.job.tool_calls",
        "limits.job.known_tokens",
        "limits.job.job_tokens",
        "limits.job.elapsed_seconds",
        "limits.job.no_progress_turns",
        "limits.job.repeat_limit",
        "limits.job.wall_clock_seconds",
        "limits.job.max_cost_usd",
    ] {
        assert_ne!(
            resolved.provenance.get(key).unwrap().source,
            SettingSource::ProjectFile,
            "untrusted project origin leaked for {key}"
        );
    }
    assert_eq!(
        resolved
            .provenance
            .get("agents.max_children")
            .unwrap()
            .source,
        SettingSource::UserFile
    );

    let trusted = resolve(
        empty()
            .with_user(user)
            .with_project(project)
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(trusted.agents.max_children, 3);
    assert_eq!(
        trusted.coding.automatic_lsp_profile.as_deref(),
        Some("other")
    );
    assert_eq!(
        trusted
            .provenance
            .get("agents.max_children")
            .unwrap()
            .source,
        SettingSource::ProjectFile
    );
    assert!(trusted.ignored_project_overrides.is_empty());
}

#[test]
fn untrusted_project_job_without_user_settings_restores_defaults() {
    let resolved = resolve(
        empty().with_project(
            ConfigFile::from_toml(
                "[agents]\nmax_children = 1\n\n[job]\nprogram_dir = '/tmp/attacker-programs'\n",
            )
            .unwrap(),
        ),
    )
    .unwrap();
    assert_eq!(resolved.agents.max_children, 3);
    assert_eq!(
        resolved.coding.program_dir, None,
        "the untrusted project directory is reverted to unset"
    );
    assert_eq!(resolved.ignored_project_overrides, ["job", "agents"]);
    for key in ["agents.max_children", "job.program_dir"] {
        assert_eq!(
            resolved.provenance.get(key).unwrap().source,
            SettingSource::Default,
            "untrusted project origin leaked for {key}"
        );
    }
}
