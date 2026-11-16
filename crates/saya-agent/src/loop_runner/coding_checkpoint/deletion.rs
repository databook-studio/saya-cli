//! Deletion is an absence observation, never a creation/retry witness.
use super::{Fact, Observation};
use crate::ToolError;
use saya_types::{Sha256Digest, WorkspacePath};
use serde_json::Value;

pub(super) fn observe(arguments: &Value, result: Result<&Value, &ToolError>) -> Observation {
    let mut observation = Observation::default();
    let possible = match result {
        Ok(_) => true,
        Err(error) => !error.proves_no_effect(),
    };
    observation.possible_mutation = possible;
    let path = arguments["path"]
        .as_str()
        .filter(|text| text.len() <= 512)
        .and_then(|text| WorkspacePath::try_from(text.to_owned()).ok());
    if possible {
        observation.invalidated_path = path.as_ref().map(|path| path.as_str().to_owned());
    }
    let Ok(value) = result else {
        return observation;
    };
    observation.omitted = 1;
    let Some(path) = path else { return observation };
    if value["path"].as_str() != Some(path.as_str())
        || value["removed"].as_bool() != Some(true)
        || saya_types::redact(path.as_str()) != path.as_str()
    {
        return observation;
    }
    let Some(digest) = value["removed_digest"].as_str() else {
        return observation;
    };
    if arguments["expected_digest"].as_str() != Some(digest) {
        return observation;
    }
    let Ok(removed_digest) = Sha256Digest::parse(digest) else {
        return observation;
    };
    observation.omitted = 0;
    observation.push(Fact::Deleted {
        path,
        removed_digest,
    });
    observation
}
