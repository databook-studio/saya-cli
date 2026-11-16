use super::{CodingApplication, CodingError};
use saya_harness::coding::integration::AssignedPaths;
use saya_types::JobId;
use serde::{Deserialize, Serialize};

impl CodingApplication {
    /// The tool selection a coding start states, canonical, or `None` when the
    /// start narrows nothing.
    ///
    /// The words are the session-only `--tools` a `/start` states. On
    /// the CLI there are none: the flag is the global one, already resolved
    /// into `runtime.resolved.tools`, and the job persists that set — so
    /// `saya start … --tools read` and `saya --tools read start …` both land
    /// here with the same list.
    ///
    /// Inside a session the words may only *narrow* the list the session
    /// itself advertises: the session has no launch to replace, so a name it
    /// does not admit is a typed error naming that name rather than a silent
    /// widening of the job's surface. With no words the job persists the
    /// session's own list, so a job started under a narrowed launch keeps that
    /// narrowing and a job started under no setting keeps the byte-identical
    /// default.
    pub(crate) fn stated_tools(
        &self,
        stated: &[String],
    ) -> Result<Option<Vec<String>>, CodingError> {
        let session = &self.runtime.resolved.tools;
        if stated.is_empty() {
            return Ok(session.stated());
        }
        let narrowed = saya_config::ResolvedTools::from_stated(Some(stated))?;
        let widening: Vec<String> = narrowed
            .allow
            .as_ref()
            .map(|allow| {
                allow
                    .names()
                    .filter(|name| !session.admits(name))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if !widening.is_empty() {
            return Err(CodingError::ToolsOutsideSession {
                names: widening
                    .iter()
                    .map(|name| format!("{name:?}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                session: session
                    .allow
                    .as_ref()
                    .map(saya_config::ToolsAllow::joined)
                    .unwrap_or_default(),
            });
        }
        Ok(narrowed.stated())
    }
}

/// User-selected job ceiling. Deserializing or displaying it grants nothing.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingJobRequest {
    pub(crate) job_id: JobId,
    pub(crate) task: String,
    pub(crate) write_paths: Vec<String>,
    /// The extension-program admission list; nothing reads
    /// this. The key is still *accepted* — `request.json` is a persisted wire
    /// shape, so a job written by an older binary must still parse on resume —
    /// but a new request never writes it, and no code path consults it.
    #[serde(default, rename = "programs", skip_serializing)]
    _legacy_programs: Vec<String>,
    #[serde(default)]
    pub(crate) database_profiles: Vec<String>,
    #[serde(default)]
    pub(crate) database_identities: std::collections::BTreeMap<String, saya_types::ProfileIdentity>,
    /// The job's own `[tools] allow` selection, in the same words the config
    /// layer and the `--tools` flag use. `None` — absent from the file — is the
    /// default: this job narrows nothing, and its `request.json` is
    /// byte-identical to one written before this key existed. A stated list is
    /// persisted canonical, so a resume and a follow-up resolve exactly the set
    /// the job started with rather than whatever the surface resolves today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tools_allow: Option<Vec<String>>,
    /// Whether this job was started plan-first: its root runs under the plan
    /// posture until an operator approves a submitted plan. `false` — every
    /// job started without `--plan` — is skipped on the wire, so a
    /// `request.json` written without plan mode is byte-identical to one
    /// written before this key existed, and a resume reads back exactly what
    /// the job started as.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) plan: bool,
}
impl CodingJobRequest {
    pub fn new(job_id: JobId, task: String, write_paths: Vec<String>) -> Result<Self, CodingError> {
        let request = Self {
            job_id,
            task,
            write_paths,
            _legacy_programs: vec![],
            database_profiles: vec![],
            database_identities: Default::default(),
            tools_allow: None,
            plan: false,
        };
        request.validate()?;
        Ok(request)
    }
    /// States that this job is plan-first. The plan posture itself is opened
    /// only by an operator's answer to a submitted plan; stating the flag
    /// grants nothing.
    pub fn with_plan(mut self, plan: bool) -> Result<Self, CodingError> {
        self.plan = plan;
        self.validate()?;
        Ok(self)
    }
    pub fn with_database_profiles(mut self, profiles: Vec<String>) -> Result<Self, CodingError> {
        self.database_profiles = profiles;
        self.validate()?;
        Ok(self)
    }
    /// States this job's tool selection. The words are validated and stored
    /// canonical, so what a job persists is exactly what it resolved — however
    /// the caller spelled it, and whatever a later resume reads back. `None`
    /// leaves the job narrowing nothing, the default every job had before the
    /// setting existed.
    pub fn with_tools_allow(mut self, tools: Option<Vec<String>>) -> Result<Self, CodingError> {
        self.tools_allow = saya_config::ResolvedTools::from_stated(tools.as_deref())?.stated();
        self.validate()?;
        Ok(self)
    }
    /// This job's tool selection, resolved. `validate` has already accepted it,
    /// so a request that loaded is a request that resolves; the error is
    /// returned rather than swallowed so a malformed list can never widen a
    /// job back to the full surface.
    pub(crate) fn tools(&self) -> Result<saya_config::ResolvedTools, CodingError> {
        Ok(saya_config::ResolvedTools::from_stated(
            self.tools_allow.as_deref(),
        )?)
    }
    pub(crate) fn validate(&self) -> Result<(), CodingError> {
        saya_config::ResolvedTools::from_stated(self.tools_allow.as_deref())?;
        let mut names = std::collections::BTreeSet::new();
        if self.database_profiles.len() > 32
            || self.database_profiles.iter().any(|name| {
                name.len() > 128 || !saya_types::is_name_shaped(name) || !names.insert(name)
            })
        {
            return Err(CodingError::InvalidRequest);
        }
        if self.task.trim().is_empty()
            || self.task.len() > 16 * 1024
            || self
                .task
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        {
            return Err(CodingError::InvalidRequest);
        }
        if !self.write_paths.is_empty() {
            AssignedPaths::parse(self.write_paths.clone()).map_err(CodingError::write_paths)?;
        }
        Ok(())
    }
    /// Whether the job's own tool selection explicitly names a write tool.
    /// `None` — the default — narrows nothing and is not an intent to write.
    pub(crate) fn states_write_tool(&self) -> Result<bool, CodingError> {
        let Some(stated) = self.tools_allow.as_deref() else {
            return Ok(false);
        };
        let tools = saya_config::ResolvedTools::from_stated(Some(stated))?;
        Ok(["write", "edit", "notebook_edit", "delete"]
            .iter()
            .any(|name| crate::agent::tools::admits(&tools, name)))
    }

    /// The approved write scope, or `None` when this job approved no path
    /// (and is therefore read-only).
    pub(crate) fn assigned_paths(&self) -> Result<Option<AssignedPaths>, CodingError> {
        if self.write_paths.is_empty() {
            return Ok(None);
        }
        AssignedPaths::parse(self.write_paths.clone())
            .map(Some)
            .map_err(CodingError::write_paths)
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }
    pub fn task(&self) -> &str {
        &self.task
    }
}
impl std::fmt::Debug for CodingJobRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingJobRequest")
            .field("job_id", &self.job_id)
            .field("write_paths", &self.write_paths.len())
            .finish_non_exhaustive()
    }
}
