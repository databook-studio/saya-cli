use serde::{Deserialize, Serialize};

/// Trusted native-coding admission mode. `Uncapped` removes configured
/// aggregate budget ceilings; resource and authority bounds remain active.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum CodingBudgetMode {
    /// Apply the configured aggregate request, token, tool, and elapsed limits.
    #[default]
    Bounded,
    /// Do not apply configured aggregate request, token, tool, or elapsed limits.
    Uncapped,
}
