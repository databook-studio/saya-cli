/// G2 property 4 — any project-layer `[host_commands]` key is a typed
/// resolve error — a model-writable file must never shape unsandboxed
/// execution (not enable it, not widen its timeout, not name its env), and
/// `--trust-project-config` does not unlock it. Moved from
/// `the_project_layer_cannot_enable_host_commands` (reason: the `enable`
/// spelling the old test pinned was deleted; the refusal keeps its bytes
/// and its hardness over every remaining key).
#[test]
fn any_project_host_commands_key_still_refuses_hard() {
    use saya_config::ResolutionInput;
    // Every remaining key refuses — shaping never rides the project layer —
    // including the deleted `enable` spelling (unknown-field deny refuses it
    // at parse, before resolve ever runs), and `--trust-project-config`
    // unlocks none of them.
    for fixture in [
        "[host_commands]\npass_env = ['CI_TOKEN']\n",
        "[host_commands]\ntimeout_seconds = 42\n",
    ] {
        let input = ResolutionInput::new(saya_config::ConnectionsFile::default())
            .with_project(saya_config::ConfigFile::from_toml(fixture).expect("fixture parses"));
        let trusted = {
            let cli = saya_config::CliOverrides {
                trust_project_config: true,
                ..Default::default()
            };
            input.clone().with_cli(cli)
        };
        for candidate in [input, trusted] {
            let error = match saya_config::resolve(candidate) {
                Err(error) => error.to_string(),
                Ok(_) => panic!("a project-layer [host_commands] must refuse: {fixture:?}"),
            };
            assert!(
                error.contains("host_commands") && error.contains("project"),
                "the refusal names the section and the layer: {error}"
            );
        }
    }
    let error = match saya_config::ConfigFile::from_toml("[host_commands]\nenable = true\n") {
        Err(error) => error.to_string(),
        Ok(_) => panic!("the deleted `enable` spelling must refuse at parse"),
    };
    assert!(
        error.contains("enable") && error.contains("pass_env"),
        "unknown-field deny names the stale key against the known keys: {error}"
    );
}
