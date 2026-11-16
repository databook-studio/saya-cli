use super::*;

/// The `[mcp_servers]` table names programs that run on the user's host, and
/// every tool those programs advertise reaches the model. It is protected
/// whole: a repository-controlled config must not be able to add a server.
/// The revert names the section, `mcp_servers`.
#[test]
fn project_layer_cannot_add_a_server() {
    let input = project_with("[mcp_servers.evil]\ncommand = \"/tmp/attacker\"\n");
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(
        resolved.mcp_servers.is_empty(),
        "the untrusted table must not add a server: {:?}",
        resolved.mcp_servers
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"mcp_servers".to_string()),
        "the report must name the protected section: {:?}",
        resolved.ignored_project_overrides
    );
}

/// The revert restores the whole table, so an untrusted layer cannot retarget
/// a server the user configured at a different program — the one case where a
/// `command` written by a repository would run in the user's name.
#[test]
fn project_layer_cannot_retarget_the_users_server() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(
            ConfigFile::from_toml("[mcp_servers.files]\ncommand = \"/usr/bin/mcp-files\"\n")
                .expect("user fixture must parse"),
        )
        .with_project(
            ConfigFile::from_toml("[mcp_servers.files]\ncommand = \"/tmp/attacker\"\n")
                .expect("project fixture must parse"),
        );
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved.mcp_servers.servers.len(),
        1,
        "the user's own server must survive an untrusted layer's table"
    );
    assert_eq!(
        resolved.mcp_servers.servers[0].command(),
        "/usr/bin/mcp-files",
        "the untrusted layer must not retarget the program"
    );
}

/// Nor remove one: an untrusted layer that states a *different* server must
/// leave the user's own server configured.
#[test]
fn project_layer_cannot_replace_the_users_servers() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(
            ConfigFile::from_toml("[mcp_servers.files]\ncommand = \"/usr/bin/mcp-files\"\n")
                .expect("user fixture must parse"),
        )
        .with_project(
            ConfigFile::from_toml("[mcp_servers.other]\ncommand = \"/tmp/attacker\"\n")
                .expect("project fixture must parse"),
        );
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved
            .mcp_servers
            .servers
            .iter()
            .map(|server| server.name())
            .collect::<Vec<_>>(),
        ["files"],
        "the user's own server must survive, and the untrusted one must not appear"
    );
}

/// `trust_project_config` is the explicit opt-in, and it is what the boundary
/// is for: with it, the project layer's server is the one that resolves. The
/// flag is the user's statement, so this is not a hole in the rule above.
#[test]
fn a_trusted_project_layer_may_state_the_table() {
    let input = project_with("[mcp_servers.files]\ncommand = \"/usr/bin/mcp-files\"\n").with_cli(
        CliOverrides {
            trust_project_config: true,
            ..Default::default()
        },
    );
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved
            .mcp_servers
            .servers
            .iter()
            .map(|server| server.name())
            .collect::<Vec<_>>(),
        ["files"]
    );
}
