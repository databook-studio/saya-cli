use std::collections::BTreeMap;

use saya_types::{DatabaseProfile, SecretRef};
use serde::Deserialize;

use crate::{
    AiProvider, ColorChoice, ConfigError, MemoryMode, OutputFormat, RedactedDiagnostics,
    ThemeChoice,
};

#[path = "model/agents.rs"]
mod agents;
#[path = "model/ai.rs"]
mod ai;
#[path = "model/config.rs"]
mod config;
#[path = "model/job.rs"]
mod job;
#[path = "model/job_keys.rs"]
mod job_keys;
#[path = "model/jobs.rs"]
mod jobs;
#[path = "model/presentation.rs"]
mod presentation;
#[path = "model/retired.rs"]
mod retired;

pub use crate::hooks::HooksFile;
pub use crate::limits::{JobLimitsFile, LimitsFile};
pub use crate::mcp::McpServersFile;
pub use crate::permissions::PermissionsFile;
pub use crate::tools::ToolsFile;
pub use crate::web_search::WebSearchFile;
pub use agents::{AgentFile, AgentRole, AgentsFile};
pub use ai::{AiFile, EndpointFile};
pub use config::{ConfigFile, ConnectionsFile, MAX_CONFIG_FILE_BYTES};
pub use job::{JobFile, JobRootMode};
pub use jobs::RunFile;
pub use jobs::{FetchJobsFile, HostCommandsFile, JobsFile, SessionCommandsFile};
pub use presentation::{InstructionsFile, MemoryFile, OutputFile, UiFile};
