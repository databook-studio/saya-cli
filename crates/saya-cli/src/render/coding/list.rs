//! `saya job list`: one row per job, with the origin in a column of its own.
//!
//! The origin is where the job was opened from — the CLI's own verb, or a chat
//! session's run. It is shown and never acted on: it says who opened a job, not
//! what the job may do.

use super::text_of;
use serde_json::Value;

/// The widest job id column, so a long id widens its own row rather than
/// pushing the state and origin columns out of line.
const JOB: usize = 32;
const STATE: usize = 12;

pub(super) fn list(all: bool, value: &Value) -> Option<String> {
    let jobs = value.get("jobs")?.as_array()?;
    let count = match jobs.len() {
        0 => "no jobs".to_owned(),
        1 => "1 job".to_owned(),
        many => format!("{many} jobs"),
    };
    let mut lines = vec![
        count,
        format!("  {:<JOB$}  {:<STATE$}  origin", "job", "state"),
    ];
    for job in jobs {
        lines.push(format!(
            "  {:<JOB$}  {:<STATE$}  {}",
            job.get("job_id")?.as_str()?,
            text_of(job.get("state")?),
            job.get("origin").map_or_else(|| "cli".to_owned(), text_of)
        ));
    }
    if !all {
        lines.push("Session jobs are hidden; add --all to list them.".into());
    }
    if value
        .get("next_cursor")
        .is_some_and(|cursor| !cursor.is_null())
    {
        lines.push("  … more jobs exist; list again after the last id".into());
    }
    Some(lines.join("\n"))
}
