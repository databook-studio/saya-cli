//! Bound enforcement for saved investigations: the checks every definition
//! must pass, however it arrived.

use std::collections::HashSet;

use super::{
    INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1,
    InvestigationError, MAX_CONNECTION_CHARS, MAX_DESCRIPTION_BYTES, MAX_FINGERPRINT_BYTES,
    MAX_NAME_CHARS, MAX_OBJECT_BYTES, MAX_OBJECTS, MAX_SQL_BYTES,
};

impl InvestigationDefinitionV1 {
    /// Re-checks every bound for a definition that may have arrived as JSON,
    /// which skips no gate; code that builds or edits a definition validates
    /// the same way before storing it.
    pub fn validate(&self) -> Result<(), InvestigationError> {
        if self.format != INVESTIGATION_FORMAT {
            return Err(InvestigationError::NotAnInvestigation);
        }
        if self.version != INVESTIGATION_FORMAT_VERSION {
            return Err(InvestigationError::UnsupportedVersion(self.version));
        }
        if self.revision < 1 {
            return Err(InvestigationError::InvalidRevision);
        }
        if self.name.trim().is_empty() || self.name.chars().count() > MAX_NAME_CHARS {
            return Err(InvestigationError::InvalidName);
        }
        if self.name.chars().any(char::is_control) {
            return Err(InvestigationError::ControlCharacter);
        }
        if let Some(description) = &self.description {
            if description.len() > MAX_DESCRIPTION_BYTES {
                return Err(InvestigationError::DescriptionTooLong);
            }
            if description.chars().any(|c| c.is_control() && c != '\n') {
                return Err(InvestigationError::ControlCharacter);
            }
        }
        if self.sql.trim().is_empty() || self.sql.len() > MAX_SQL_BYTES {
            return Err(InvestigationError::InvalidSql);
        }
        let connection_ok = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-');
        if !(1..=MAX_CONNECTION_CHARS).contains(&self.connection.len())
            || !self.connection.bytes().all(connection_ok)
        {
            return Err(InvestigationError::InvalidConnection);
        }
        if self.objects.len() > MAX_OBJECTS {
            return Err(InvestigationError::TooManyObjects(self.objects.len()));
        }
        for object in &self.objects {
            if object.is_empty() || object.len() > MAX_OBJECT_BYTES {
                return Err(InvestigationError::InvalidObject);
            }
            if object.chars().any(char::is_control) {
                return Err(InvestigationError::ControlCharacter);
            }
        }
        let mut seen = HashSet::new();
        if !self.objects.iter().all(|object| seen.insert(object)) {
            return Err(InvestigationError::DuplicateObject);
        }
        if let Some(fingerprint) = &self.schema_fingerprint {
            if fingerprint.is_empty() || fingerprint.chars().any(char::is_control) {
                return Err(InvestigationError::InvalidFingerprint);
            }
            if fingerprint.len() > MAX_FINGERPRINT_BYTES {
                return Err(InvestigationError::InvalidFingerprint);
            }
        }
        if self.updated_unix_ms < self.created_unix_ms {
            return Err(InvestigationError::UpdatedBeforeCreated);
        }
        Ok(())
    }
}
