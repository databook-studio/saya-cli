//! The shape one `subagent_type` composes as: the registry's role and write
//! flag, mapped onto the harness role the child is composed with, plus the
//! model that type's helpers ask for and the shell mode that shape runs under.
//!
//! A shape is a type, never a grant. The child's authority is still narrowed
//! per child from the job's own scope (`ChildAuthorityView`), so a helper type
//! can only decide what a helper does with authority the job already had.

use saya_agent::ApprovalPolicy;
use saya_config::{AgentRole, HelperAgents};
use saya_harness::coding::policy::ChildRole;

use super::CodingError;

/// The role and write capability one helper type composes as, and the model it
/// sends when its config names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelperShape {
    pub(crate) role: ChildRole,
    pub(crate) write: bool,
    /// The model this type's helpers ask for. `None` — the default — is the
    /// job's own model, so the request is the one a helper always sent.
    pub(crate) model: Option<String>,
}

impl HelperShape {
    /// The mode a helper of this shape runs its `shell` under: a helper that
    /// cannot write gets a sandbox-only shell with nothing writable whatever
    /// the job's mode; a writing helper follows the job's Ask/Bypass rule.
    pub(crate) fn shell_policy(&self, job_policy: ApprovalPolicy) -> ApprovalPolicy {
        if self.write {
            job_policy
        } else {
            ApprovalPolicy::ReadOnly
        }
    }
}

/// The shape one helper type composes as. An unknown type is a typed refusal
/// naming the types this job does advertise.
pub(crate) fn helper_shape(
    agents: &HelperAgents,
    subagent_type: &str,
) -> Result<HelperShape, CodingError> {
    let agent = agents
        .get(subagent_type)
        .ok_or_else(|| CodingError::UnknownSubagentType {
            requested: subagent_type.to_owned(),
            known: agents.known_names(),
        })?;
    Ok(HelperShape {
        role: match agent.role {
            AgentRole::Analyst => ChildRole::Analyst,
            AgentRole::Implementer => ChildRole::Implementer,
        },
        write: agent.allow_write,
        model: agent.model.clone(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::{HelperShape, helper_shape};
    use crate::commands::coding::CodingError;
    use saya_agent::ApprovalPolicy;
    use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
    use saya_harness::coding::policy::ChildRole;

    /// The registry a config resolves to, through the same public path the
    /// application uses.
    fn agents(toml: &str) -> saya_config::HelperAgents {
        resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("the fixture parses")),
        )
        .expect("the fixture resolves")
        .agents
        .types
    }

    /// The shape one advertised type composes as.
    fn shape(agents: &saya_config::HelperAgents, name: &str) -> HelperShape {
        helper_shape(agents, name).expect("the type is advertised")
    }

    /// A job with no `[agents]` table composes exactly the two built-in
    /// shapes: `explore` reads only, `general` writes.
    #[test]
    fn the_builtins_are_read_only_explore_and_writing_general() {
        let agents = agents("");
        assert_eq!(
            shape(&agents, "explore"),
            HelperShape {
                role: ChildRole::Analyst,
                write: false,
                model: None,
            },
            "an explore helper holds no write capability"
        );
        assert_eq!(
            shape(&agents, "general"),
            HelperShape {
                role: ChildRole::Implementer,
                write: true,
                model: None,
            },
            "a general helper writes the shared working checkout"
        );
    }

    /// A configured analyst-shaped type is read-only and a configured
    /// implementer-shaped one writes: adding a helper type is configuration.
    #[test]
    fn a_configured_agent_composes_the_shape_it_declares() {
        let agents =
            agents("[agents.scout]\nrole = 'analyst'\n\n[agents.builder]\nrole = 'implementer'\n");
        assert_eq!(
            shape(&agents, "scout"),
            HelperShape {
                role: ChildRole::Analyst,
                write: false,
                model: None,
            },
            "a declared analyst reads only"
        );
        assert_eq!(
            shape(&agents, "builder"),
            HelperShape {
                role: ChildRole::Implementer,
                write: true,
                model: None,
            },
            "a declared implementer writes"
        );
    }

    /// A declared entry may narrow a built-in: `general` can be made
    /// read-only, and the type keeps the name the model already sends.
    #[test]
    fn a_declared_entry_narrows_a_builtin() {
        let agents = agents("[agents.general]\nallow_write = false\n");
        assert_eq!(
            shape(&agents, "general"),
            HelperShape {
                role: ChildRole::Implementer,
                write: false,
                model: None,
            },
            "a narrowed general keeps its role and loses the write"
        );
    }

    /// A type's `model` is part of what the registry says about it, so the
    /// shape carries it: composition reads one lookup and one truth. A type
    /// that states no model carries none, which is what leaves its request
    /// exactly the one it always was.
    #[test]
    fn a_configured_type_carries_its_model_and_an_unset_one_carries_none() {
        let agents = agents("[agents.general]\nmodel = 'helper-model'\n");
        assert_eq!(
            shape(&agents, "general").model.as_deref(),
            Some("helper-model"),
            "the shape carries the type's own model"
        );
        assert_eq!(
            shape(&agents, "explore").model,
            None,
            "a type that states no model carries none"
        );
        assert_eq!(
            shape(&agents, "general").role,
            ChildRole::Implementer,
            "a model changes which model answers, never the shape"
        );
    }

    /// An unknown type is refused by name, and the refusal names the types the
    /// job does advertise.
    #[test]
    fn an_unknown_type_is_refused_with_the_known_types() {
        let agents = agents("[agents.scout]\nrole = 'analyst'\n");
        let refusal = helper_shape(&agents, "reviewer").expect_err("an unknown type is refused");
        assert!(
            matches!(
                &refusal,
                CodingError::UnknownSubagentType { requested, known }
                    if requested == "reviewer" && known == "explore, general, scout"
            ),
            "the refusal is typed and names the known types: {refusal:?}"
        );
        let message = refusal.to_string();
        for known in ["explore", "general", "scout"] {
            assert!(
                message.contains(known),
                "the refusal names every known type: {message}"
            );
        }
    }

    /// A helper that cannot write runs a read-only shell whatever the job's
    /// mode; a writing helper follows the job's Ask/Bypass rule.
    #[test]
    fn a_read_only_shape_has_a_read_only_shell_whatever_the_job_mode() {
        let writing = HelperShape {
            role: ChildRole::Implementer,
            write: true,
            model: None,
        };
        assert_eq!(
            writing.shell_policy(ApprovalPolicy::Bypass),
            ApprovalPolicy::Bypass
        );
        assert_eq!(
            writing.shell_policy(ApprovalPolicy::Ask),
            ApprovalPolicy::Ask
        );
        for job_policy in [
            ApprovalPolicy::Ask,
            ApprovalPolicy::Bypass,
            ApprovalPolicy::ReadOnly,
        ] {
            for read_only in [
                HelperShape {
                    role: ChildRole::Analyst,
                    write: false,
                    model: None,
                },
                HelperShape {
                    role: ChildRole::Implementer,
                    write: false,
                    model: None,
                },
            ] {
                assert_eq!(
                    read_only.shell_policy(job_policy),
                    ApprovalPolicy::ReadOnly,
                    "a helper without the write capability gets a read-only shell"
                );
            }
        }
    }
}
