use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

pub(crate) const MAX_CONTROL_RECORD_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum ControlWireError {
    #[error("control record is {0} bytes, over the {MAX_CONTROL_RECORD_BYTES}-byte limit")]
    Oversize(usize),
    #[error("unsupported control record version {0}")]
    UnsupportedVersion(u32),
    #[error("control record is malformed")]
    Malformed,
}

/// Requires a private strict record type whose Deserialize path validates all
/// nested fields. This helper only bounds bytes and rejects missing/future
/// versions before that type's strict decode runs.
pub(crate) fn decode_versioned<T: DeserializeOwned>(
    bytes: &[u8],
    supported_version: u32,
) -> Result<T, ControlWireError> {
    if bytes.len() > MAX_CONTROL_RECORD_BYTES {
        return Err(ControlWireError::Oversize(bytes.len()));
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| ControlWireError::Malformed)?;
    match probe.version {
        Some(version) if version != supported_version => {
            Err(ControlWireError::UnsupportedVersion(version))
        }
        Some(_) => serde_json::from_slice(bytes).map_err(|_| ControlWireError::Malformed),
        None => Err(ControlWireError::Malformed),
    }
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ControlWireError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ControlWireError::Malformed)?;
    if bytes.len() > MAX_CONTROL_RECORD_BYTES {
        return Err(ControlWireError::Oversize(bytes.len()));
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct VersionProbe {
    version: Option<u32>,
}
