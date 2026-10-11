use std::{collections::BTreeMap, path::PathBuf};

use saya_types::{DatabaseProfile, SecretRef};
use serde::Deserialize;

use crate::{
    AiProvider, ColorChoice, ConfigError, MemoryMode, OutputFormat, RedactedDiagnostics,
    ThemeChoice,
};

#[path = "model/ai.rs"]
mod ai;
#[path = "model/coding.rs"]
mod coding;
#[path = "model/config.rs"]
mod config;
#[path = "model/jobs.rs"]
mod jobs;
#[path = "model/presentation.rs"]
mod presentation;

pub use ai::{AiFile, EndpointFile};
pub use coding::{CheckProfileFile, CodingFile};
pub use config::{ConfigFile, ConnectionsFile, MAX_CONFIG_FILE_BYTES};
pub use jobs::RunFile;
pub use jobs::{
    FetchJobsFile, HostCommandsFile, InterpreterJobsFile, JobsFile, RunnerJobsFile,
    SessionCommandsFile,
};
pub use presentation::{InstructionsFile, MemoryFile, OutputFile, UiFile};
