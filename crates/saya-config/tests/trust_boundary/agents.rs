use super::*;

/// The top-level `[agents]` table decides which helper types a coding job's
/// `helpers` tool advertises, how each one composes, and the two bounds its
/// scheduler runs under. It is protected whole, so a repository-controlled
/// config can neither add a type nor retarget a built-in nor raise the child
/// bound: the revert names the section, `agents`.
#[test]
fn project_layer_cannot_add_or_retarget_a_helper_agent() {
    let input = project_with(
        "[agents]\nmax_children = 3\n\n[agents.sneaky]\nrole = 'implementer'\n\n\
         [agents.general]\nallow_write = false\n",
    );
    let resolved = resolve(input).expect("resolution succeeds");
    let builtin = saya_config::HelperAgents::builtin();
    for name in ["explore", "general"] {
        assert_eq!(
            resolved.agents.types.get(name),
            builtin.get(name),
            "the untrusted entry must not retarget the built-in {name}"
        );
    }
    assert!(
        resolved.agents.types.get("sneaky").is_none(),
        "the untrusted table must not add a helper type"
    );
    assert_eq!(
        resolved.agents.max_children,
        saya_config::ResolvedAgents::default().max_children,
        "the untrusted table must not raise the child bound"
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"agents".to_string()),
        "the report must name the protected section: {:?}",
        resolved.ignored_project_overrides
    );
}

/// A project-layer entry cannot give a helper type a model either: `model`
/// decides which model answers a helper, and it rides in on the same protected
/// table as the role. A trusted layer may still set it.
#[test]
fn project_layer_cannot_set_a_helper_agent_model() {
    let input = project_with("[agents.general]\nmodel = \"attacker-model\"\n");
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved
            .agents
            .types
            .get("general")
            .and_then(|agent| agent.model.as_deref()),
        None,
        "the untrusted entry must not retarget the built-in's model"
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"agents".to_string()),
        "the report must name the protected section: {:?}",
        resolved.ignored_project_overrides
    );

    let user = ConfigFile::from_toml("[agents.general]\nmodel = \"fast\"\n").expect("parses");
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user)
            .with_project(ConfigFile::from_toml("[run]\nmax_rows = 7\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert_eq!(
        resolved
            .agents
            .types
            .get("general")
            .and_then(|agent| agent.model.as_deref()),
        Some("fast"),
        "the trusted layer's model resolves"
    );
}

/// The same table from the trusted layers resolves, and explicit trust
/// (`--trust-project-config`) restores the project's declaration — the
/// protected list is a default about who speaks first, not a judgment that
/// project config is useless.
#[test]
fn trusted_layers_may_declare_helper_agents() {
    let user = ConfigFile::from_toml("[agents.reviewer]\nrole = 'analyst'\n").expect("parses");
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user)
            .with_project(ConfigFile::from_toml("[run]\nmax_rows = 7\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert!(
        resolved.agents.types.get("reviewer").is_some(),
        "the trusted layer's type resolves"
    );
    assert!(resolved.ignored_project_overrides.is_empty());

    let input = project_with("[agents.scout]\nrole = 'analyst'\n").with_cli(CliOverrides {
        trust_project_config: true,
        ..Default::default()
    });
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(
        resolved.agents.types.get("scout").is_some(),
        "explicit trust honours the project's declaration"
    );
    assert!(resolved.ignored_project_overrides.is_empty());
}
