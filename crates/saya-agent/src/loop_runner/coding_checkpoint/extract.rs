use super::{Fact, Observation};
use crate::ToolError;
use saya_types::{AgentId, InvocationId, ManagedCheckoutId, Sha256Digest, WorkspacePath};
use serde_json::Value;

fn path(value: &Value) -> Option<WorkspacePath> {
    let path = value.as_str()?;
    if path.len() > 512 {
        return None;
    }
    WorkspacePath::try_from(path.to_owned()).ok()
}
fn public_identity(value: &str) -> bool {
    saya_types::redact(value) == value
}

fn missing(error: &ToolError) -> bool {
    match error {
        ToolError::WorkspaceFileMissing => true,
        ToolError::NoEffect(inner) => missing(inner),
        _ => false,
    }
}

pub(in crate::loop_runner) fn observe(
    name: &str,
    arguments: &Value,
    result: Result<&Value, &ToolError>,
) -> Observation {
    if name == "delete" {
        return super::deletion::observe(arguments, result);
    }
    // A possible mutation invalidates earlier existence AND absence even when
    // identity metadata is omitted. It does not supply a creation witness.
    let mut observation = Observation {
        possible_mutation: matches!(name, "write" | "edit")
            && match result {
                Ok(_) => true,
                Err(error) => !error.proves_no_effect(),
            },
        ..Observation::default()
    };
    if observation.possible_mutation {
        observation.invalidated_path =
            path(&arguments["path"]).map(|path| path.as_str().to_owned());
    }
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            if name == "read" && missing(error) {
                observation.missing_path =
                    path(&arguments["path"]).map(|path| path.as_str().to_owned());
            }
            return observation;
        }
    };
    match name {
        "read" | "write" | "edit" => {
            observation.omitted = 1;
            let Some(path) = path(&value["path"]) else {
                return observation;
            };
            if arguments["path"].as_str() != Some(path.as_str()) {
                return observation;
            }
            let Some(digest) = value["digest"]
                .as_str()
                .and_then(|digest| Sha256Digest::parse(digest).ok())
            else {
                return observation;
            };
            let Some(size) = value["size"].as_u64() else {
                return observation;
            };
            let operation = match name {
                "write" => "write",
                "edit" => "edit",
                _ => "read",
            };
            if operation != "read" {
                observation.mutation_path = Some(path.as_str().to_owned());
            }
            // A masked path is not an alternate identity for the original file.
            if public_identity(path.as_str()) {
                observation.omitted = 0;
                observation.push(Fact::File {
                    path,
                    digest,
                    size,
                    operation,
                });
            }
        }
        "follow_up_child" => {
            if let Some(fact) = follow_up(value) {
                observation.push(fact);
            } else {
                observation.omitted = 1;
            }
        }
        _ => {}
    }
    observation
}

fn follow_up(value: &Value) -> Option<Fact> {
    let agent = value["agent_id"].as_str()?;
    let previous = value["previous_invocation_id"].as_str()?;
    let invocation = value["invocation_id"].as_str()?;
    let checkout = value["checkout_id"].as_str()?;
    if [agent, previous, invocation, checkout]
        .iter()
        .any(|identity| identity.len() > 128 || !public_identity(identity))
    {
        return None;
    }
    Some(Fact::FollowUp {
        agent_id: AgentId::parse(agent).ok()?,
        previous_invocation_id: InvocationId::parse(previous).ok()?,
        invocation_id: InvocationId::parse(invocation).ok()?,
        previous_completion_sequence: value["previous_completion_sequence"]
            .as_u64()
            .filter(|sequence| *sequence > 0)?,
        checkout_id: ManagedCheckoutId::parse(checkout).ok()?,
    })
}
