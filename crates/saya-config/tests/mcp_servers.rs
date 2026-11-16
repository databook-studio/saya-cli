//! The `[mcp_servers]` table: what it accepts, what it refuses, and what a
//! resolved server carries.

use saya_config::{
    ConfigFile, ConnectionsFile, MAX_MCP_ARGS, MAX_MCP_SERVERS, ResolutionInput, resolve,
};

fn resolved(toml: &str) -> saya_config::ResolvedConfig {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("fixture must parse"));
    resolve(input).expect("resolution succeeds")
}

fn refusal(toml: &str) -> String {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("fixture must parse"));
    resolve(input)
        .expect_err("resolution must refuse")
        .to_string()
}

/// No table means no server: the default is off, and nothing downstream has to
/// ask whether the setting was stated.
#[test]
fn no_table_resolves_to_no_server() {
    let resolved = resolved("");
    assert!(resolved.mcp_servers.is_empty());
    assert!(resolved.mcp_servers.servers.is_empty());
}

/// A stated server resolves to exactly what it wrote, with the two timeouts
/// defaulted.
#[test]
fn a_stated_server_resolves_with_its_defaults() {
    let resolved = resolved(
        "[mcp_servers.files]\ncommand = \"mcp-files\"\nargs = [\"--root\", \".\"]\n\
         env = { FILES_TOKEN = { env = \"SAYA_FILES_TOKEN\" } }\n",
    );
    let server = &resolved.mcp_servers.servers[0];
    assert_eq!(server.name(), "files");
    assert_eq!(server.command(), "mcp-files");
    assert_eq!(server.args(), ["--root", "."]);
    assert_eq!(server.env().len(), 1);
    assert_eq!(
        server.startup_timeout(),
        std::time::Duration::from_millis(saya_config::DEFAULT_MCP_STARTUP_TIMEOUT_MS)
    );
    assert_eq!(
        server.call_timeout(),
        std::time::Duration::from_millis(saya_config::DEFAULT_MCP_CALL_TIMEOUT_MS)
    );
}

/// Servers resolve in name order, so two runs of one config compose the same
/// advertisement and the same tool list order.
#[test]
fn servers_resolve_in_name_order() {
    let resolved =
        resolved("[mcp_servers.zulu]\ncommand = \"z\"\n\n[mcp_servers.alpha]\ncommand = \"a\"\n");
    let names: Vec<&str> = resolved
        .mcp_servers
        .servers
        .iter()
        .map(|server| server.name())
        .collect();
    assert_eq!(names, ["alpha", "zulu"]);
}

/// A remote server is out of scope for this release. The refusal names the
/// field and the fix, rather than parsing the entry into a server that would
/// never start.
#[test]
fn a_remote_server_is_refused_by_name() {
    let reason = refusal("[mcp_servers.remote]\nurl = \"https://mcp.example/sse\"\n");
    assert!(
        reason.contains("mcp_servers.remote.url") && reason.contains("command"),
        "the refusal must name the field and the fix: {reason}"
    );
}

/// An entry with no command names nothing to start.
#[test]
fn a_server_without_a_command_is_refused() {
    let reason = refusal("[mcp_servers.files]\nargs = [\"--root\"]\n");
    assert!(
        reason.contains("command is required"),
        "the refusal must say what is missing: {reason}"
    );
}

/// The server name is half of every tool name it contributes, so its shape is
/// checked: the double underscore separates server from tool and may not
/// appear inside a name.
#[test]
fn a_server_name_holding_the_separator_is_refused() {
    let reason = refusal("[mcp_servers.\"a__b\"]\ncommand = \"x\"\n");
    assert!(
        reason.contains("a__b"),
        "the refusal must name the entry: {reason}"
    );
}

/// The name is also what the operator writes in `[tools] allow`, so it is
/// restricted to the lowercase alphabet a tool name may carry.
#[test]
fn an_uppercase_server_name_is_refused() {
    let reason = refusal("[mcp_servers.Files]\ncommand = \"x\"\n");
    assert!(reason.contains("Files"), "{reason}");
}

/// The argv count is bounded: a server is one program and a bounded list of
/// words.
#[test]
fn an_oversized_argv_is_refused() {
    let args: Vec<String> = (0..=MAX_MCP_ARGS).map(|i| format!("\"a{i}\"")).collect();
    let reason = refusal(&format!(
        "[mcp_servers.files]\ncommand = \"x\"\nargs = [{}]\n",
        args.join(", ")
    ));
    assert!(reason.contains("args"), "{reason}");
}

/// The count of servers is bounded, and the refusal says how many were named.
#[test]
fn more_servers_than_the_limit_is_refused() {
    let mut toml = String::new();
    for index in 0..=MAX_MCP_SERVERS {
        toml.push_str(&format!("[mcp_servers.s{index}]\ncommand = \"x\"\n\n"));
    }
    let reason = refusal(&toml);
    assert!(reason.contains("servers"), "{reason}");
}

/// A timeout outside its range is refused, not clamped: a clamp would run
/// under a bound the user did not write.
#[test]
fn a_timeout_over_the_ceiling_is_refused() {
    let reason = refusal(&format!(
        "[mcp_servers.files]\ncommand = \"x\"\nstartup_timeout_ms = {}\n",
        saya_config::MAX_MCP_STARTUP_TIMEOUT_MS + 1
    ));
    assert!(reason.contains("startup_timeout_ms"), "{reason}");
}

/// An `env` value must be a reference. A literal is refused by name, with the
/// fix, rather than as serde's undiagnosable "invalid type: string".
#[test]
fn an_inline_env_secret_is_refused_with_the_fix() {
    let error = ConfigFile::from_toml(
        "[mcp_servers.files]\ncommand = \"x\"\nenv = { TOKEN = \"hunter2\" }\n",
    )
    .expect_err("a literal secret is refused");
    let message = error.to_string();
    assert!(
        message.contains("mcp_servers `files`: env.TOKEN") && message.contains("{ env ="),
        "the refusal must name the location and the fix: {message}"
    );
}

/// The env variable name must be a name a process can be handed.
#[test]
fn an_env_name_that_is_not_a_variable_name_is_refused() {
    let reason =
        refusal("[mcp_servers.files]\ncommand = \"x\"\nenv = { \"1BAD\" = { env = \"X\" } }\n");
    assert!(reason.contains("1BAD"), "{reason}");
}
