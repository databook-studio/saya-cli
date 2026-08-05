use std::collections::BTreeMap;

use crate::{AiProvider, ConfigFile, ConnectionsFile};

/// Command-line values which have the highest configuration precedence.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub profile: Option<String>,
    pub provider: Option<AiProvider>,
    pub model: Option<String>,
    pub allow_data_sharing: Option<bool>,
    pub max_rows: Option<usize>,
}

/// Explicit configuration inputs. It never reads a `.env` file implicitly.
#[derive(Debug, Clone)]
pub struct ResolutionInput {
    pub(crate) connections: ConnectionsFile,
    pub(crate) user: Option<ConfigFile>,
    pub(crate) project: Option<ConfigFile>,
    pub(crate) env_file: BTreeMap<String, String>,
    pub(crate) process_env: BTreeMap<String, String>,
    pub(crate) cli: CliOverrides,
}

impl ResolutionInput {
    pub fn new(connections: ConnectionsFile) -> Self {
        Self {
            connections,
            user: None,
            project: None,
            env_file: BTreeMap::new(),
            process_env: BTreeMap::new(),
            cli: CliOverrides::default(),
        }
    }

    pub fn with_user(mut self, value: ConfigFile) -> Self {
        self.user = Some(value);
        self
    }

    pub fn with_project(mut self, value: ConfigFile) -> Self {
        self.project = Some(value);
        self
    }

    pub fn with_env_file<I, K, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env_file = values
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self
    }

    pub fn with_process_env<I, K, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.process_env = values
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self
    }

    pub fn with_cli(mut self, value: CliOverrides) -> Self {
        self.cli = value;
        self
    }
}
