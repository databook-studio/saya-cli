//! The `[agents]` registry rules: what a declared entry may add, what it may
//! narrow, and what it may never widen.

use super::{HelperAgent, HelperAgents, resolve};
use crate::{AgentRole, ConfigError, ConfigFile, MAX_ENDPOINT_STRING_CHARS};

/// The registry one `[agents]` table resolves to.
fn registry(toml: &str) -> HelperAgents {
    let file = ConfigFile::from_toml(toml).expect("the fixture parses");
    resolve(&file.agents.types).expect("the fixture resolves")
}

/// The registry a fixture resolves to, or the refusal it earns.
fn refused(toml: &str) -> ConfigError {
    ConfigFile::from_toml(toml)
        .map(|file| resolve(&file.agents.types))
        .expect("the fixture parses")
        .expect_err("the fixture is refused")
}

fn agent(agents: &HelperAgents, name: &str) -> HelperAgent {
    agents
        .get(name)
        .expect("the type is in the registry")
        .clone()
}

/// A config that declares no entry leaves the job exactly as it was: the two
/// built-ins, `explore` read-only and `general` write-capable, neither of them
/// carrying a model of its own.
#[test]
fn an_absent_table_keeps_the_two_builtins() {
    let agents = registry("[agents]\nmax_children = 2\n");
    assert_eq!(
        agent(&agents, "explore"),
        HelperAgent {
            role: AgentRole::Analyst,
            allow_write: false,
            model: None,
        }
    );
    assert_eq!(
        agent(&agents, "general"),
        HelperAgent {
            role: AgentRole::Implementer,
            allow_write: true,
            model: None,
        }
    );
    assert_eq!(agents.known_names(), "explore, general");
}

/// A declared type is added, and starts read-only: a new helper type must say
/// `implementer` to write.
#[test]
fn a_declared_type_is_added_and_starts_read_only() {
    let agents = registry("[agents.reviewer]\n");
    assert_eq!(
        agent(&agents, "reviewer"),
        HelperAgent {
            role: AgentRole::Analyst,
            allow_write: false,
            model: None,
        }
    );
    assert_eq!(agents.known_names(), "explore, general, reviewer");
}

/// A declared `implementer` writes, and its write flag follows its role.
#[test]
fn a_declared_implementer_writes() {
    let agents = registry("[agents.builder]\nrole = 'implementer'\n");
    assert_eq!(
        agent(&agents, "builder"),
        HelperAgent {
            role: AgentRole::Implementer,
            allow_write: true,
            model: None,
        }
    );
}

/// An entry states only what it changes: an empty entry for a built-in leaves
/// that built-in exactly as it was.
#[test]
fn an_entry_states_only_what_it_changes() {
    let agents = registry("[agents.general]\n\n[agents.explore]\n");
    assert_eq!(
        agent(&agents, "general"),
        agent(&HelperAgents::builtin(), "general")
    );
    assert_eq!(
        agent(&agents, "explore"),
        agent(&HelperAgents::builtin(), "explore")
    );
}

/// A declared entry may narrow a built-in: `general` can lose its write, and a
/// `general`-shaped type can become an analyst.
#[test]
fn a_declared_entry_narrows_a_builtin() {
    let agents = registry("[agents.general]\nallow_write = false\n");
    assert_eq!(
        agent(&agents, "general"),
        HelperAgent {
            role: AgentRole::Implementer,
            allow_write: false,
            model: None,
        }
    );

    let agents = registry("[agents.general]\nrole = 'analyst'\n");
    assert_eq!(
        agent(&agents, "general"),
        HelperAgent {
            role: AgentRole::Analyst,
            allow_write: false,
            model: None,
        }
    );
}

/// A type's `model` is its own: the helpers of that type ask for it, and a type
/// that states none carries none — the job's own model, unchanged.
#[test]
fn a_declared_model_belongs_to_the_type_that_states_it() {
    let agents = registry(
        "[agents.general]\nmodel = \"helper-model\"\n\n[agents.scout]\nmodel = \"reader-model\"\n",
    );
    assert_eq!(
        agent(&agents, "general").model.as_deref(),
        Some("helper-model"),
        "the type states the model its helpers send"
    );
    assert_eq!(
        agent(&agents, "scout").model.as_deref(),
        Some("reader-model")
    );
    assert_eq!(
        agent(&agents, "explore").model,
        None,
        "a type that states no model carries none"
    );
    assert_eq!(
        agent(&agents, "general").role,
        AgentRole::Implementer,
        "a model changes which model answers, never the shape"
    );
}

/// A model name is carried into a provider request, so it is bounded exactly as
/// an endpoint's model is.
#[test]
fn an_over_long_model_is_refused() {
    let long = "m".repeat(MAX_ENDPOINT_STRING_CHARS + 1);
    let error = refused(&format!("[agents.general]\nmodel = \"{long}\"\n"));
    assert!(
        matches!(
            &error,
            ConfigError::AgentModelTooLong { name, value, max }
                if name == "general"
                    && *value == MAX_ENDPOINT_STRING_CHARS + 1
                    && *max == MAX_ENDPOINT_STRING_CHARS
        ),
        "the refusal names the entry and the bound: {error}"
    );

    let at_bound = "m".repeat(MAX_ENDPOINT_STRING_CHARS);
    let agents = registry(&format!("[agents.general]\nmodel = \"{at_bound}\"\n"));
    assert_eq!(
        agent(&agents, "general").model.as_deref().map(str::len),
        Some(MAX_ENDPOINT_STRING_CHARS),
        "a model at the bound is admitted"
    );
}

/// A declared entry may never widen a type: `explore` cannot be made an
/// implementer, and a read-only type cannot be given the write back.
#[test]
fn a_declared_entry_cannot_widen_a_builtin() {
    for toml in [
        "[agents.explore]\nrole = 'implementer'\n",
        "[agents.explore]\nallow_write = true\n",
    ] {
        assert!(
            matches!(refused(toml), ConfigError::InvalidAgentType { .. }),
            "{toml} widens a built-in and must be refused"
        );
    }
}

/// An analyst reads only: `allow_write = true` on one is refused by name
/// rather than silently ignored, so the config says what the helper does.
#[test]
fn an_analyst_cannot_be_granted_write() {
    let error = refused("[agents.reviewer]\nallow_write = true\n");
    assert!(
        matches!(
            &error,
            ConfigError::InvalidAgentType { name, reason }
                if name == "reviewer" && reason.contains("reads only")
        ),
        "the refusal names the entry and why: {error}"
    );
}

/// A helper type name is advertised in the tool schema, so it is a bounded
/// lowercase identifier.
#[test]
fn a_type_name_is_a_bounded_lowercase_identifier() {
    for name in ["Reviewer", "9lives", "read-only!", ""] {
        let toml = format!("[agents.\"{name}\"]\n");
        assert!(
            matches!(refused(&toml), ConfigError::InvalidAgentType { .. }),
            "{name:?} is not a usable type name"
        );
    }
    let long = "a".repeat(33);
    assert!(
        matches!(
            refused(&format!("[agents.{long}]\n")),
            ConfigError::InvalidAgentType { .. }
        ),
        "a name over the character bound is refused"
    );
}

/// The two setting names are reserved: a key in `[agents]` means one thing, and
/// a type named `max_children` would make it mean two. A table built in code
/// cannot slip one past the text check either.
#[test]
fn a_reserved_setting_name_is_never_a_helper_type() {
    let mut declared = std::collections::BTreeMap::new();
    declared.insert("max_children".to_owned(), crate::AgentFile::default());
    let error = resolve(&declared).expect_err("a reserved name is refused");
    assert!(
        matches!(
            &error,
            ConfigError::InvalidAgentType { name, reason }
                if name == "max_children" && reason.contains("reserved")
        ),
        "the refusal names the reserved key: {error}"
    );
}

/// The table is bounded like every other config list: the registry is
/// advertised whole in the `helpers` schema.
#[test]
fn the_table_is_bounded() {
    let declared: String = (0..17)
        .map(|index| format!("[agents.type{index}]\n"))
        .collect();
    assert!(
        matches!(
            ConfigFile::from_toml(&declared)
                .map(|file| resolve(&file.agents.types))
                .expect("the fixture parses"),
            Err(ConfigError::SettingAboveMaximum {
                field: "agents",
                value: 17,
                ..
            })
        ),
        "a table over the entry bound is refused"
    );
}

/// An entry carries only the fields this version reads: a typo is a parse
/// error, not a silently ignored narrowing.
#[test]
fn an_unknown_entry_field_is_refused() {
    assert!(
        ConfigFile::from_toml("[agents.reviewer]\nwrites = true\n").is_err(),
        "an unknown field in an agent entry is refused"
    );
}
