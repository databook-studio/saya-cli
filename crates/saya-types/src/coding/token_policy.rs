//! Optional verified upper bounds; callers must validate external provenance.
use super::Sha256Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenBudgetPolicyData {
    pub version: u32,
    pub total_tokens: u64,
    pub provider: String,
    pub model: String,
    pub endpoint_sha256: Sha256Digest,
    pub verification_sha256: Sha256Digest,
    pub input_context_upper_bound: u64,
    pub output_tokens: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TokenBudgetPolicyV1(TokenBudgetPolicyData);
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid token budget policy or request binding")]
pub struct TokenBudgetError;
impl TokenBudgetPolicyV1 {
    /// Trusted host only: the digest must identify independently verified exact
    /// provider/model/endpoint bounds. Shape validation does not verify that proof.
    pub fn new(data: TokenBudgetPolicyData) -> Result<Self, TokenBudgetError> {
        if data.version != 1
            || data.total_tokens == 0
            || data.input_context_upper_bound == 0
            || data.output_tokens == 0
            || data.provider.is_empty()
            || data.provider.len() > 64
            || !data
                .provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || data.model.is_empty()
            || data.model.len() > 2048
            || data.model.contains('\0')
            || data
                .input_context_upper_bound
                .checked_add(u64::from(data.output_tokens))
                .is_none_or(|sum| sum > data.total_tokens)
        {
            return Err(TokenBudgetError);
        }
        Ok(Self(data))
    }
    pub fn data(&self) -> &TokenBudgetPolicyData {
        &self.0
    }
    pub fn canonical_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("fixed policy fields serialize")
    }
    pub fn digest(&self) -> Sha256Digest {
        hash(&self.canonical_json_bytes())
    }
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, TokenBudgetError> {
        if bytes.len() > 8192 {
            return Err(TokenBudgetError);
        }
        serde_json::from_slice(bytes).map_err(|_| TokenBudgetError)
    }
    pub fn matches_request(
        &self,
        provider: &str,
        model: &str,
        endpoint: &str,
        output_tokens: u32,
    ) -> bool {
        endpoint.len() <= 8192
            && self.0.provider == provider
            && self.0.model == model
            && self.0.endpoint_sha256 == hash(endpoint.as_bytes())
            && self.0.output_tokens == output_tokens
    }
    pub fn endpoint_digest(endpoint: &str) -> Result<Sha256Digest, TokenBudgetError> {
        if endpoint.is_empty() || endpoint.len() > 8192 {
            return Err(TokenBudgetError);
        }
        Ok(hash(endpoint.as_bytes()))
    }
}
fn hash(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::parse(&format!("{:x}", Sha256::digest(bytes)))
        .expect("SHA256 hex has fixed valid shape")
}
impl<'de> Deserialize<'de> for TokenBudgetPolicyV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(TokenBudgetPolicyData::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}
