use std::{fmt, marker::PhantomData};

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
    check_version(bytes, supported_version)?;
    serde_json::from_slice(bytes).map_err(|_| ControlWireError::Malformed)
}

pub(crate) fn check_version(bytes: &[u8], supported_version: u32) -> Result<(), ControlWireError> {
    if bytes.len() > MAX_CONTROL_RECORD_BYTES {
        return Err(ControlWireError::Oversize(bytes.len()));
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| ControlWireError::Malformed)?;
    match probe.version {
        Some(version) if version != supported_version => {
            Err(ControlWireError::UnsupportedVersion(version))
        }
        Some(_) => Ok(()),
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

/// A nullable ID field that must be present in the wire record. Deserializing
/// through `deserialize_any` distinguishes a missing field from explicit null.
pub(crate) struct RequiredNullableId<T>(pub(crate) Option<T>);

impl<'de, T> Deserialize<'de> for RequiredNullableId<T>
where
    T: TryFrom<String>,
    T::Error: fmt::Display,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct NullableIdVisitor<T>(PhantomData<T>);

        impl<'de, T> serde::de::Visitor<'de> for NullableIdVisitor<T>
        where
            T: TryFrom<String>,
            T::Error: fmt::Display,
        {
            type Value = RequiredNullableId<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a validated ID string or null")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(RequiredNullableId(None))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(RequiredNullableId(None))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                T::try_from(value.to_owned())
                    .map(Some)
                    .map(RequiredNullableId)
                    .map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                T::try_from(value)
                    .map(Some)
                    .map(RequiredNullableId)
                    .map_err(E::custom)
            }
        }

        deserializer.deserialize_any(NullableIdVisitor(PhantomData))
    }
}
