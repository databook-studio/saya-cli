//! The config overrides the global flags state. They live beside the grammar
//! they belong to, and nowhere else: each launch surface folds the same
//! [`GlobalOptions`](super::GlobalOptions) through these methods.

use super::GlobalOptions;

impl GlobalOptions {
    /// The `[limits.job]` keys the `--coding-max-*` flags state. The flag
    /// names are unchanged; only the keys they write moved.
    pub(crate) fn job_overrides(&self) -> saya_config::JobLimitsFile {
        fn dimension(value: Option<u64>) -> Option<toml::Value> {
            value
                .and_then(|value| i64::try_from(value).ok())
                .map(toml::Value::Integer)
        }
        saya_config::JobLimitsFile {
            logical_requests: dimension(
                self.coding_max_logical_model_requests
                    .map(|value| value as u64),
            ),
            transport_attempts: dimension(
                self.coding_max_physical_transport_attempts
                    .map(|value| value as u64),
            ),
            job_tokens: dimension(self.coding_max_job_tokens),
            elapsed_seconds: dimension(self.coding_max_elapsed_seconds),
            max_cost_usd: self.max_cost_usd.map(toml::Value::Float),
            ..Default::default()
        }
    }

    /// The `[agents]` settings a flag states, and nothing else: no flag names
    /// a helper type, so the registry stays whatever the config layers say.
    pub(crate) fn agents_overrides(&self) -> saya_config::AgentsFile {
        saya_config::AgentsFile {
            max_children: self.agents_max_children,
            max_concurrent_model_requests: self.agents_max_concurrent_model_requests,
            ..Default::default()
        }
    }

    /// The `--tools` allow list as a config override: stated whole, or not at
    /// all — an absent flag leaves every config layer's `[tools] allow` in
    /// force.
    pub(crate) fn tools_overrides(&self) -> saya_config::ToolsFile {
        saya_config::ToolsFile {
            allow: (!self.tools.is_empty()).then(|| self.tools.clone()),
        }
    }
}
