use super::*;

/// Resolves `[jobs]`, which holds only the download budgets a run's
/// `http_download` calls spend from. Numeric bounds are checked here, at
/// resolve time, with typed errors — never silently clamped at the point of
/// use.
pub(crate) fn resolve(file: &JobsFile) -> Result<ResolvedJobs, ConfigError> {
    Ok(ResolvedJobs {
        fetch: resolve_fetch(file.fetch.as_ref())?,
    })
}

/// Resolves `[jobs.fetch]`: each declared key is checked against the
/// minimum discipline at resolve time, with typed errors — never silently
/// clamped at the point of use. Every key is optional; the defaults are the
/// provisional conservative numbers documented above.
fn resolve_fetch(fetch: Option<&FetchJobsFile>) -> Result<ResolvedFetchJobs, ConfigError> {
    let fetch = fetch.cloned().unwrap_or_default();
    let max_file_bytes = fetch.max_file_bytes.unwrap_or(FETCH_MAX_FILE_BYTES);
    require_at_least_one("fetch.max_file_bytes", max_file_bytes)?;
    let max_run_bytes = fetch.max_run_bytes.unwrap_or(FETCH_MAX_RUN_BYTES);
    require_at_least_one("fetch.max_run_bytes", max_run_bytes)?;
    let timeout_seconds = fetch.timeout_seconds.unwrap_or(FETCH_TIMEOUT_SECONDS);
    require_at_least_one("fetch.timeout_seconds", timeout_seconds)?;
    Ok(ResolvedFetchJobs {
        max_file_bytes,
        max_run_bytes,
        timeout_seconds,
    })
}
