use super::CodingError;
use saya_harness::coding::integration::AssignedPaths;
use saya_types::{JobId, RunnerScope};
use serde::{Deserialize, Serialize};

/// User-selected job ceiling. Deserializing or displaying it grants nothing.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingJobRequest {
    pub(crate) job_id: JobId,
    pub(crate) task: String,
    pub(crate) write_paths: Vec<String>,
    pub(crate) programs: Vec<String>,
}
impl CodingJobRequest {
    pub fn new(
        job_id: JobId,
        task: String,
        write_paths: Vec<String>,
        programs: Vec<String>,
    ) -> Result<Self, CodingError> {
        let request = Self {
            job_id,
            task,
            write_paths,
            programs,
        };
        request.validate()?;
        Ok(request)
    }
    pub(crate) fn validate(&self) -> Result<(), CodingError> {
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
            AssignedPaths::parse(self.write_paths.clone())
                .map_err(|_| CodingError::InvalidRequest)?;
        }
        if !self.programs.is_empty() {
            RunnerScope::new(self.programs.clone()).map_err(|_| CodingError::InvalidRequest)?;
        }
        Ok(())
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
            .field("programs", &self.programs.len())
            .finish_non_exhaustive()
    }
}
