use serde::{Deserialize, Serialize};

/// The closed set of structured diagnostic formats supported by coding checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum CheckDiagnosticFormat {
    RustcJson,
}
