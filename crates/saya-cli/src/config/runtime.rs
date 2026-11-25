use super::sources::Paths;
use crate::cli::{GlobalOptions, ThemeArg};
use saya_config::{CliOverrides, ConnectionsFile, ResolutionInput, resolve};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Pure investigations-root resolution: the `SAYA_INVESTIGATIONS_DIR` env
/// override wins; otherwise the documents sit beside the state database.
/// The one resolver the composition uses — operations read the composed
/// [`RuntimeConfig::investigations_root`] field and never the environment.
pub(crate) fn investigations_root_from(env: Option<&OsStr>, state_db: &Path) -> PathBuf {
    if let Some(path) = env {
        return PathBuf::from(path);
    }
    state_db
        .parent()
        .map(|dir| dir.join("investigations"))
        .unwrap_or_else(|| PathBuf::from("investigations"))
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("{0}")]
    Config(#[from] saya_config::ConfigError),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("explicit path does not exist: {0}")]
    Missing(PathBuf),
    #[error("invalid approval mode: {0}")]
    Approval(String),
}

#[derive(Clone)]
pub struct RuntimeConfig {
    pub resolved: saya_config::ResolvedConfig,
    pub connections: ConnectionsFile,
    pub config_path: Option<PathBuf>,
    pub connections_path: Option<PathBuf>,
    pub cache_scope: PathBuf,
    /// Where saved-investigation documents live, resolved once here at
    /// composition (`SAYA_INVESTIGATIONS_DIR`, else beside the state DB —
    /// per `adr-0004`). Every in-process caller builds its repository from this
    /// field and never re-reads the environment at operation time, so tests
    /// point a composed runtime at a private temp root.
    pub investigations_root: PathBuf,
    pub(crate) secret_values: BTreeMap<String, String>,
}

pub fn load(options: &GlobalOptions, cwd: &Path) -> Result<RuntimeConfig, RuntimeError> {
    let user_dir = super::sources::user_config_dir();
    let process = super::sources::process_env();
    let runtime = match load_with_provider(options, cwd, &user_dir, process.clone(), None) {
        Ok(runtime) => runtime,
        // Two or more provider keys and no stated provider: ask the operator
        // when there is a terminal to ask on, and otherwise report the
        // ambiguity. The prompt's answer is a decision, so the retry states
        // it rather than letting detection run again.
        Err(
            error @ RuntimeError::Config(saya_config::ConfigError::AmbiguousProviderKeys { .. }),
        ) => {
            let Some(key) = super::auto_provider::prompt_for_provider(&process) else {
                return Err(error);
            };
            load_with_provider(options, cwd, &user_dir, process, Some(key))?
        }
        Err(error) => return Err(error),
    };
    super::auto_provider::announce(&runtime);
    warn_ignored_project_overrides(&runtime);
    warn_invalid_permission_rules(&runtime);
    Ok(runtime)
}

/// A refused `[permissions] allow`/`deny` entry is reported, not dropped
/// silently: an inert rule the operator believes is in force is worse than a
/// loud one. The rules that did parse are still in force.
fn warn_invalid_permission_rules(runtime: &RuntimeConfig) {
    for diagnostic in &runtime.resolved.permissions.rules.diagnostics {
        eprintln!("warning: {diagnostic}");
    }
}

/// The project layer is untrusted: when it tried to change security-critical
/// settings, say so instead of silently ignoring the attempt.
fn warn_ignored_project_overrides(runtime: &RuntimeConfig) {
    let ignored = &runtime.resolved.ignored_project_overrides;
    if ignored.is_empty() {
        return;
    }
    // One short line, every run. It cannot be less frequent without knowing
    // whether this invocation is interactive, which `load` is not told — and
    // it should not be silent, because the project layer trying to redirect an
    // API key is worth saying out loud. The detail lives in `config doctor`,
    // which is where someone goes to find out what is wrong with their setup;
    // repeating four key names on every command trains people to skip the line.
    let count = ignored.len();
    let noun = if count == 1 { "setting" } else { "settings" };
    eprintln!(
        "warning: ignored {count} security-critical {noun} from this project's \
         .saya/config.toml — run `saya config doctor` for which, and why."
    );
}

pub fn load_with_sources(
    options: &GlobalOptions,
    cwd: &Path,
    user_dir: &Path,
    process: BTreeMap<String, String>,
) -> Result<RuntimeConfig, RuntimeError> {
    load_with_provider(options, cwd, user_dir, process, None)
}

/// The loader's body, with the provider prompt's answer when there is one.
/// `None` is the ordinary path: the environment's keys may be consulted when
/// no layer stated a provider.
fn load_with_provider(
    options: &GlobalOptions,
    cwd: &Path,
    user_dir: &Path,
    process: BTreeMap<String, String>,
    prompted: Option<&'static saya_config::ProviderKey>,
) -> Result<RuntimeConfig, RuntimeError> {
    // Anchor discovery at load time. Later application composition may use a
    // different cwd; keep the selected source path (including project symlinks)
    // rather than relabelling it by the canonical target's location.
    let absolute_cwd = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| RuntimeError::Read {
                path: cwd.to_path_buf(),
                source,
            })?
            .join(cwd)
    };
    // Missing source directories historically resolve defaults. Anchor their
    // lexical absolute path too, without introducing an existence requirement.
    let source_cwd = absolute_cwd.canonicalize().unwrap_or(absolute_cwd);
    let paths = Paths::discover(&source_cwd, user_dir);
    let user = super::sources::read_config(&paths.user_config)?;
    let selected_config = options
        .config
        .as_ref()
        .or_else(|| paths.project_config.as_ref().filter(|path| path.exists()))
        .or_else(|| paths.user_config.as_ref().filter(|path| path.exists()));
    let project = match options.config.as_ref() {
        Some(path) => Some(super::sources::read_required_config(path)?),
        None => super::sources::read_config(&paths.project_config)?,
    };
    let explicit_connections = options
        .connections
        .as_ref()
        .map(|path| source_cwd.join(path));
    let selected_connections = explicit_connections
        .as_ref()
        .or_else(|| {
            paths
                .project_connections
                .as_ref()
                .filter(|path| path.exists())
        })
        .or_else(|| paths.user_connections.as_ref().filter(|path| path.exists()));
    let connections =
        super::sources::read_connections(selected_connections, options.connections.is_some())?;
    let env_file = match options.env_file.as_ref() {
        Some(path) => super::sources::read_env_file(path)?,
        None => BTreeMap::new(),
    };
    let mut secret_values = env_file.clone();
    secret_values.extend(process.clone());
    // The investigations root is resolved here, once: the env override comes
    // from the process-env map this loader is given (in production the real
    // environment), the default from beside the state-DB path. After this,
    // every operation reads the composed field and never the environment.
    let investigations_root = investigations_root_from(
        process
            .get("SAYA_INVESTIGATIONS_DIR")
            .map(|value| OsStr::new(value.as_str())),
        &crate::state_path::state_db_path(),
    );
    let provider = match options.provider.as_deref() {
        Some(raw) => Some(saya_config::AiProvider::parse(raw).ok_or_else(|| {
            RuntimeError::Config(saya_config::ConfigError::Parse(format!(
                "invalid --provider '{raw}' (expected ollama, openai, openai_compatible, anthropic, or gemini)"
            )))
        })?),
        None => None,
    };
    let input = ResolutionInput::new(connections.clone())
        .with_user(user.unwrap_or_default())
        .with_project(project.unwrap_or_default())
        .with_env_file(env_file)
        .with_process_env(process)
        .with_cli(CliOverrides {
            profile: options.profile.clone(),
            provider,
            prompted_provider: prompted,
            model: options.model.clone(),
            allow_data_sharing: if options.no_data_sharing {
                Some(false)
            } else {
                options.allow_data_sharing.then_some(true)
            },
            max_rows: options.max_rows,
            candidates: options.candidates,
            show_thinking: if options.show_thinking {
                Some(true)
            } else {
                None
            },
            theme: options.theme.map(ThemeArg::to_choice),
            color: options.no_color.then_some(saya_config::ColorChoice::Never),
            job: options.job_overrides(),
            agents: options.agents_overrides(),
            tools: options.tools_overrides(),
            permissions: saya_config::PermissionsFile {
                allow_bypass: None,
                allow: (!options.allow_command.is_empty()).then(|| options.allow_command.clone()),
                deny: (!options.deny_command.is_empty()).then(|| options.deny_command.clone()),
                ..Default::default()
            },
            trust_project_config: options.trust_project_config,
        });
    let resolved = resolve(input)?;
    let cache_scope = super::scope::resolve(selected_connections, &source_cwd);
    Ok(RuntimeConfig {
        resolved,
        connections,
        config_path: selected_config.cloned(),
        connections_path: selected_connections.cloned(),
        cache_scope,
        investigations_root,
        secret_values,
    })
}

impl std::fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeConfig")
            .field("resolved", &self.resolved)
            .field("connections", &self.connections)
            .field("config_path", &self.config_path)
            .field("connections_path", &self.connections_path)
            .field("cache_scope", &"[redacted]")
            .field("investigations_root", &self.investigations_root)
            .field("secret_values", &"[redacted]")
            .finish()
    }
}

/// The consent posture this invocation states, and the flag that stated it:
/// `--mode` is the one axis, `--approval-mode` its one-release alias. `None`
/// where neither is stated, so the caller's default (or `--non-interactive`)
/// answers instead.
///
/// `--permission-mode` is not read here: it states the *session's* mode, which
/// moves the task posture too, and the launch applies it as a mode statement
/// (`mode::stated_mode`) rather than as a posture.
pub fn stated_posture(options: &GlobalOptions) -> Option<(&'static str, &str)> {
    if let Some(value) = options.mode.as_deref() {
        return Some(("--mode", value));
    }
    options
        .approval_mode
        .as_deref()
        .map(|value| ("--approval-mode", value))
}

pub fn approval_mode(options: &GlobalOptions) -> Result<saya_agent::ApprovalPolicy, RuntimeError> {
    let (flag, spelling) = match stated_posture(options) {
        Some((flag, value)) => (flag, value),
        None if options.non_interactive => ("--non-interactive", "never"),
        None => ("--mode", "ask"),
    };
    let mode =
        crate::interactive::session::permissions::mode::parse(spelling).ok_or_else(|| {
            RuntimeError::Approval(format!(
                "invalid {flag} `{spelling}`: expected ask, accept-edits, plan, bypass, read-only, \
             or never"
            ))
        })?;
    match mode.decompose().0 {
        Some(policy) => Ok(policy),
        // `plan` names no posture of its own: it needs an operator to answer
        // the plan it submits, and a headless surface has nobody to ask.
        // Refused in a sentence rather than read as a default.
        None => Err(RuntimeError::Approval(format!(
            "`{flag} plan` needs the interactive session to answer the plan: drop it, or launch \
             the session with `saya {flag} plan`"
        ))),
    }
}

/// The session-launch default posture, in the one spelling the state carries.
///
/// This reads `--approval-mode` only, and deliberately: `--mode` (and its
/// `--permission-mode` alias) states the *mode*, which moves the task posture
/// too, so the launch applies it afterwards as a mode statement
/// ([`mode::stated_mode`]) instead of folding it into the defaults.
///
/// [`mode::stated_mode`]: crate::interactive::session::permissions::mode::stated_mode
pub fn approval_name(options: &GlobalOptions) -> Result<String, RuntimeError> {
    let policy = match options.approval_mode.as_deref() {
        Some(value) => value
            .parse()
            .map_err(|error: saya_agent::ApprovalPolicyParseError| {
                RuntimeError::Approval(error.to_string())
            })?,
        None if options.non_interactive => saya_agent::ApprovalPolicy::Never,
        None => saya_agent::ApprovalPolicy::Ask,
    };
    Ok(match policy {
        saya_agent::ApprovalPolicy::Ask => "ask",
        saya_agent::ApprovalPolicy::AcceptEdits => "accept-edits",
        saya_agent::ApprovalPolicy::ReadOnly => "read-only",
        saya_agent::ApprovalPolicy::Never => "never",
        saya_agent::ApprovalPolicy::Bypass => "bypass",
    }
    .into())
}

/// Whether this process's configuration allows the `bypass` permission mode
/// to be reached from a session — **the one source** of that answer, so the
/// `/mode` switch, the keyboard cycle and the status bar cannot disagree.
/// The launch reads it once onto [`SessionState::allow_bypass`], and every
/// later reader reads that field.
///
/// The key is `[permissions] allow_bypass`, and the table is protected whole:
/// a project layer that states it is reverted before resolution, so this
/// reads the trusted layers only. Unset is `false` — the closed direction —
/// and a session's own `--approval-mode bypass` is unaffected, because that
/// is the process stating the consent itself.
///
/// [`SessionState::allow_bypass`]:
///     crate::interactive::session::state::SessionState::allow_bypass
pub(crate) fn allow_bypass(runtime: &RuntimeConfig) -> bool {
    runtime.resolved.permissions.allow_bypass
}

pub fn format_name(
    options: &GlobalOptions,
    resolved: &saya_config::ResolvedConfig,
) -> crate::render::RenderFormat {
    if options.format != crate::cli::FormatArg::Text {
        options.format.into()
    } else {
        resolved.output_format.into()
    }
}

/// A test-only investigations root under the temp dir: the initializer every
/// hand-built test runtime uses, so no fixture can resolve into the machine's
/// real data directory. Uniquely named per call — two fixtures never share a
/// root, and nothing is created until something writes.
#[cfg(test)]
pub(crate) fn temp_investigations_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-investigations-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
