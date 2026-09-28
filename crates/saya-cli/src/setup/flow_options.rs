//! Everything the flow needs, injectable so tests drive it without a
//! terminal: the CLI options, the directories, the environment, the reload
//! closure (`None` builds the real `load_with_sources` check), and the
//! probes.

use std::collections::BTreeMap;

use crate::cli::GlobalOptions;

use super::probe::FlowProbes;

/// The reload check `commit` runs after publishing the files.
pub(crate) type Reload = Box<dyn FnMut() -> Result<(), String> + 'static>;

pub(crate) struct FlowOptions {
    pub(crate) options: GlobalOptions,
    pub(crate) user_dir: std::path::PathBuf,
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) reload: Option<Reload>,
    pub(crate) probes: FlowProbes,
}

impl FlowOptions {
    /// The real options: the user config directory, the process environment,
    /// the real probes, and the real reload at commit time.
    pub(crate) fn real(options: GlobalOptions) -> Self {
        let env = crate::config::sources::process_env();
        Self {
            probes: FlowProbes::real(&env),
            env,
            user_dir: crate::config::sources::user_config_dir(),
            cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            options,
            reload: None,
        }
    }
}
