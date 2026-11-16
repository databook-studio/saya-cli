//! The one `[tools] allow` / `--tools` list: what it resolves to, what it
//! refuses, and who may state it.
//!
//! One setting governs every agent surface. It only ever narrows, so the tests
//! here are about the set's own meaning — validation, canonicalisation,
//! precedence, and the untrusted-layer boundary — not about any one surface.

use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, ResolvedTools, ToolsAllow,
    ToolsAllowError, ToolsFile, resolve,
};

fn input() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

fn with_user(toml: &str) -> ResolutionInput {
    input().with_user(ConfigFile::from_toml(toml).expect("user fixture must parse"))
}

fn with_project(toml: &str) -> ResolutionInput {
    input().with_project(ConfigFile::from_toml(toml).expect("project fixture must parse"))
}

fn with_cli_tools(words: &[&str]) -> ResolutionInput {
    input().with_cli(CliOverrides {
        tools: ToolsFile {
            allow: Some(words.iter().map(|word| (*word).to_string()).collect()),
        },
        ..Default::default()
    })
}

/// A valid list resolves to exactly the names it stated, canonical and sorted.
/// `bash` is the model's spelling of `shell`, so it resolves to `shell`.
#[test]
fn a_valid_list_resolves_canonically() {
    let resolved = resolve(with_user(
        "[tools]\nallow = [\"write\", \"bash\", \"read\"]\n",
    ))
    .expect("a valid list resolves");
    let allow = resolved.tools.allow.clone().expect("the list was stated");
    assert_eq!(
        allow.names().collect::<Vec<_>>(),
        ["read", "shell", "write"]
    );
    assert!(resolved.tools.admits("read"));
    assert!(
        resolved.tools.admits("bash"),
        "the alias names the same tool"
    );
    assert!(!resolved.tools.admits("edit"));
}

/// Duplicates state one intent. Repeating a name is not worth failing a launch
/// over, so the set is de-duplicated rather than refused.
#[test]
fn duplicates_are_de_duplicated() {
    let allow =
        ToolsAllow::parse(["read", "read", " bash ", "shell"]).expect("duplicates are fine");
    assert_eq!(allow.names().collect::<Vec<_>>(), ["read", "shell"]);
}

/// An unknown word is refused with the valid names, so the user can fix it
/// without reading the docs.
#[test]
fn an_unknown_name_is_a_typed_error_naming_the_valid_ones() {
    let error = resolve(with_user("[tools]\nallow = [\"reed\"]\n")).expect_err("unknown name");
    let saya_config::ConfigError::ToolsAllow(ToolsAllowError::Unknown { name }) = &error else {
        panic!("expected the typed unknown-name error, got {error:?}");
    };
    assert_eq!(name, "reed");
    let text = error.to_string();
    assert!(text.contains("\"reed\""), "{text}");
    assert!(text.contains("read"), "the valid names ride along: {text}");
}

/// An empty list is not "allow everything": it is a typo. Treating it as the
/// default would advertise a surface the user asked to narrow.
#[test]
fn an_empty_list_is_a_typed_error() {
    let error = resolve(with_user("[tools]\nallow = []\n")).expect_err("empty list");
    assert!(
        matches!(
            error,
            saya_config::ConfigError::ToolsAllow(ToolsAllowError::Empty)
        ),
        "expected the typed empty-list error, got {error:?}"
    );
    assert!(error.to_string().contains("at least one tool"), "{error}");
}

/// `helpers` alone is not a list. The root delegates, so a helper runs the
/// job's set minus `helpers` — and a list of `helpers` alone derives an empty
/// helper set, which admits nothing. Stating it is a typo, and the launch
/// refuses it rather than handing every helper no tools at all.
#[test]
fn helpers_alone_is_a_typed_error() {
    for words in [vec!["helpers"], vec!["helpers", "helpers"]] {
        let error = resolve(with_cli_tools(&words))
            .expect_err("a list of `helpers` alone leaves every helper empty");
        let text = error.to_string();
        assert!(
            text.contains("at least one other tool"),
            "{words:?}: the refusal says what is missing: {text}"
        );
        assert!(
            text.contains("helpers"),
            "{words:?}: the refusal names the tool: {text}"
        );
    }
    // One more governed name is all it takes for the derived set to be useful.
    let resolved = resolve(with_cli_tools(&["helpers", "read"])).expect("helpers plus a tool");
    assert!(resolved.tools.for_helper().admits("read"));
}

/// The connection-tied tools are not governed: their availability follows the
/// selected connection or profile. Naming one is refused with that reason
/// rather than silently doing nothing.
#[test]
fn a_database_tool_name_is_a_typed_error() {
    for name in ["schema_discovery", "bounded_sql_query", "contract_search"] {
        let error = resolve(with_user(&format!("[tools]\nallow = [\"{name}\"]\n")))
            .expect_err("a database tool cannot be named");
        assert!(
            matches!(
                error,
                saya_config::ConfigError::ToolsAllow(ToolsAllowError::DatabaseTool { .. })
            ),
            "{name}: expected the typed database-tool error, got {error:?}"
        );
        assert!(
            error.to_string().contains("follow"),
            "{name}: the refusal must say what decides them: {error}"
        );
    }
}

/// The default is no list at all, and no list admits everything: a surface
/// under it advertises exactly what it composes.
#[test]
fn an_unstated_list_admits_everything() {
    let resolved = resolve(input()).expect("resolution succeeds");
    assert_eq!(resolved.tools, ResolvedTools::default());
    assert!(resolved.tools.allow.is_none());
    for name in saya_config::GOVERNED_TOOLS {
        assert!(resolved.tools.admits(name), "{name} is admitted by default");
    }
    assert!(resolved.tools.admits("schema_discovery"));
}

/// The flag is the operator's own word at launch: it overrides every config
/// layer, including a trusted user layer's list.
#[test]
fn the_flag_overrides_the_config_layer() {
    let resolved = resolve(
        with_user("[tools]\nallow = [\"read\", \"write\"]\n").with_cli(CliOverrides {
            tools: ToolsFile {
                allow: Some(vec!["grep".into()]),
            },
            ..Default::default()
        }),
    )
    .expect("resolution succeeds");
    let allow = resolved
        .tools
        .allow
        .clone()
        .expect("the flag stated a list");
    assert_eq!(allow.names().collect::<Vec<_>>(), ["grep"]);
    assert!(!resolved.tools.admits("read"));
}

/// The flag's words are validated exactly as the config layer's are, so a
/// mistyped flag fails the launch rather than silently widening the surface.
#[test]
fn a_mistyped_flag_is_refused() {
    assert!(resolve(with_cli_tools(&["reed"])).is_err());
    assert!(resolve(with_cli_tools(&["schema_discovery"])).is_err());
}

/// A repository's `.saya/config.toml` is untrusted input: it must not be able
/// to widen the surface a user narrowed, nor narrow away the tools the user's
/// own work needs. The whole `[tools]` table is reverted and reported.
#[test]
fn the_project_layer_cannot_set_the_tools_allow_list() {
    let resolved =
        resolve(with_project("[tools]\nallow = [\"read\"]\n")).expect("resolution succeeds");
    assert_eq!(
        resolved.tools,
        ResolvedTools::default(),
        "the untrusted list must be reverted"
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"tools".to_string()),
        "the report must name the protected section: {:?}",
        resolved.ignored_project_overrides
    );
}

/// The same table from a trusted layer resolves, and explicit trust restores
/// the project's declaration — the protected list is about who speaks first,
/// not a judgment that project config is useless.
#[test]
fn trusted_layers_may_set_the_tools_allow_list() {
    let resolved = resolve(with_user("[tools]\nallow = [\"read\"]\n"))
        .expect("a trusted layer may state the list");
    assert!(
        resolved
            .tools
            .allow
            .clone()
            .expect("the list was stated")
            .allows("read")
    );

    let trusted = with_project("[tools]\nallow = [\"grep\"]\n").with_cli(CliOverrides {
        trust_project_config: true,
        ..Default::default()
    });
    let resolved = resolve(trusted).expect("resolution succeeds");
    assert!(
        resolved
            .tools
            .allow
            .clone()
            .expect("the trusted project list resolved")
            .allows("grep")
    );
}

/// A helper child runs the job's set minus `helpers`: only the root delegates.
/// An unstated list stays unstated, so the helper's own composition is what
/// narrows it.
#[test]
fn a_helper_set_is_the_jobs_minus_helpers() {
    let stated = ResolvedTools::from_stated(Some(&["helpers".to_string(), "read".to_string()]))
        .expect("a valid list");
    let helper = stated.for_helper();
    assert!(!helper.admits("helpers"));
    assert!(helper.admits("read"));
    assert_eq!(helper.stated(), Some(vec!["read".to_string()]));

    assert_eq!(
        ResolvedTools::default().for_helper(),
        ResolvedTools::default()
    );
}

/// A persisted job states its set in the same words the config layer does, and
/// reads back canonical: the round trip is what makes a resume run the set the
/// job started with.
#[test]
fn a_stated_set_round_trips_canonically() {
    let stated =
        ResolvedTools::from_stated(Some(&["bash".into(), "read".into()])).expect("a valid list");
    assert_eq!(
        stated.stated(),
        Some(vec!["read".to_string(), "shell".to_string()])
    );
    assert_eq!(
        ResolvedTools::from_stated(stated.stated().as_deref()).expect("re-reads"),
        stated
    );
    assert_eq!(
        ResolvedTools::from_stated(None).expect("absent"),
        ResolvedTools::default()
    );
}
#[path = "tools_allow/vocabulary.rs"]
mod vocabulary;
