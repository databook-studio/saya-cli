use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A validated lowercase SHA-256 digest. It identifies content but does not
/// authenticate it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn parse(value: &str) -> Result<Self, DigestError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(DigestError::InvalidSha256);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The lowercase-hex SHA-256 of `bytes` — the shape [`Sha256Digest::parse`]
/// validates. Identifies content; authenticates nothing.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl TryFrom<String> for Sha256Digest {
    type Error = DigestError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// Why a digest does not use the lowercase SHA-256 wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DigestError {
    #[error("digest must be 64 lowercase hexadecimal characters")]
    InvalidSha256,
}
