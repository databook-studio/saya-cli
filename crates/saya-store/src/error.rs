//! Typed, payload-free store errors.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    #[error("local state store is unavailable")]
    Unavailable,
    #[error("the requested record does not exist")]
    NotFound,
    #[error("the record conflicts with an existing record")]
    Conflict,
    #[error("the value exceeds a store limit")]
    LimitExceeded,
    #[error("the value is not valid for storage")]
    Invalid,
    #[error("the state database was written by a newer version of saya")]
    VersionUnsupported,
    /// The store's path can never be opened — a parent component is a regular
    /// file, the filesystem denies access, or it is read-only. Permanent, not
    /// the transient lock contention `Unavailable` exists for, so the opener
    /// fails fast instead of retrying for the busy ceiling.
    #[error("the state store cannot be opened at its path")]
    OpenFailed,
}

impl StoreError {
    pub fn unavailable() -> Self {
        Self::Unavailable
    }
    pub fn not_found() -> Self {
        Self::NotFound
    }
    pub fn conflict() -> Self {
        Self::Conflict
    }
    pub fn limit_exceeded() -> Self {
        Self::LimitExceeded
    }
    pub fn invalid() -> Self {
        Self::Invalid
    }
}

#[cfg(test)]
mod tests {
    use super::StoreError;

    #[test]
    fn errors_are_payload_free_and_fieldless() {
        let errors = [
            StoreError::Unavailable,
            StoreError::NotFound,
            StoreError::Conflict,
            StoreError::LimitExceeded,
            StoreError::Invalid,
            StoreError::VersionUnsupported,
            StoreError::OpenFailed,
        ];
        for error in errors {
            let rendered = error.to_string();
            assert!(!rendered.is_empty());
            assert!(!rendered.contains('{'));
            assert!(!rendered.contains(':'));
            assert!(!rendered.contains("SUPERSECRETVALUE"));
        }
    }
}
