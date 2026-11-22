//! Opening the session's run for the record it was resumed from.
//!
//! A `saya --resume <id>` session starts with no run, and the record it loads
//! names one. [`crate::chat::decide`] says what that link means — attach it,
//! prepare its next epoch, open read-only, or refuse it — and this module acts
//! on that one answer: a link this process can take is resumed here and driven
//! like any other run, a record that opens read-only says why and gets a run of
//! its own, and a job that cannot be resumed states its one sentence and the
//! process leaves.
//!
//! **One decision, one resume.** The seed the resumed run is prepared with is
//! the one the same decision states, so a new epoch carries the kept tail and
//! an attached run carries none — the run it already is.

use std::path::Path;
use std::sync::Arc;

use saya_agent::{CancellationToken, NoopEventSink};
use saya_store::{RedactedSession, SqliteStateStore};
use saya_types::{JobId, JobState};

use super::open::session_run;
use super::resume::{self, Facts, Opening};
use super::run::Run;
use super::{LineChat, LineSession, open};
use crate::chat::{ChatFront, RunComposition};
use crate::commands::coding::{
    CodingApplication, CodingAuthorization, Launch, NativeCodingJob, TurnInputs,
};
use crate::interactive::session::lifecycle::resume::block_on;

/// What the front did with the record's link.
pub(crate) enum Begun {
    /// The run is open, and the line is a turn of it. `said` is the one line
    /// the record itself states first — why it opened read-only.
    Open {
        /// The session driving the run.
        chat: Box<LineChat>,
        /// The sentence the front says before the line's own output.
        said: Option<String>,
    },
    /// The job cannot be resumed: one sentence, and the process leaves.
    Refused {
        /// The sentence, as the core words it.
        sentence: String,
    },
}

impl LineChat {
    /// Opens the session for the record it resumed, following the link's
    /// decision: the run it names, a run of the record's own, or the refusal.
    pub(crate) fn begin(
        parts: LineSession,
        record: RedactedSession,
        store: &SqliteStateStore,
    ) -> Result<Begun, String> {
        let root = super::session_host::state_root(&parts)?;
        let opening = resume::opening(&record, facts(&root, &record, store));
        match opening {
            Opening::Refused { sentence } => Ok(Begun::Refused { sentence }),
            Opening::ReadOnly { reason } => Ok(Begun::Open {
                chat: Box::new(LineChat::open(parts)?),
                said: Some(reason),
            }),
            Opening::Attach { job } => resumed(parts, &job, Vec::new()),
            Opening::NewEpoch { job, .. } => {
                // The decision states *whether* a run is seeded; the conversation
                // it is seeded with is the one the session holds — its kept tail
                // behind the compaction boundary, summary first — the same pairs
                // a composed run is opened with.
                let seed = parts.seed.clone();
                resumed(parts, &job, seed)
            }
        }
    }
}

/// The session over the job the engine hands back: its next epoch, prepared
/// with `seed` and driven by the same front every composed run gets.
fn resumed(parts: LineSession, job: &JobId, seed: Vec<(String, String)>) -> Result<Begun, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("the session's runtime could not be built: {error}"))?;
    let line = runtime.block_on(resume_and_open(parts, job, seed))?;
    Ok(Begun::Open {
        chat: Box::new(LineChat::new(line, runtime)),
        said: None,
    })
}

/// Resumes the linked job and opens the session's run over it.
async fn resume_and_open(
    parts: LineSession,
    job: &JobId,
    seed: Vec<(String, String)>,
) -> Result<super::line::Line, String> {
    let (application, shape) = super::session_host::open_host(&parts).await?;
    // The session's mode, stated on the host the resume composes its job over: a
    // resumed run turns under the posture the session holds now, exactly as a
    // composed one does.
    let application = Launch::Attached(parts.mode).compose(application);
    // The surface's own fact, stated beside the operator the resume attaches:
    // a front with nobody to ask advertises no write-shaped call, and the
    // resumed run states exactly what a composed one does.
    application.with_approval_surface(parts.can_prompt);
    let job = resume_job(&application, job).await?;
    // The resumed job's approval re-derives the scopes the run it continues was
    // opened with. A session's grants die with the process that stated them, so
    // the session's own statement is made last: a resumed session restarts
    // unstated, exactly as a composed one does.
    application.with_launch_grants(parts.grants.clone());
    let (front, metrics) = open::front_for(&parts);
    let stated = session_run(
        &parts,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        parts.session.clone(),
        &shape,
    )
    .await?;
    let plugins = RunComposition::resumed_plugins(&application, &job, stated)
        .map_err(|error| error.to_string())?;
    job.run_prepare(
        &NoopEventSink,
        TurnInputs {
            seed,
            plugins: Some(plugins),
            tasks: parts.tasks.clone(),
            facts: parts.facts.clone(),
            ..TurnInputs::default()
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let run = Run::resumed(Arc::clone(&job), &front, metrics.clone()).await;
    Ok(super::line::Line::new(
        application,
        parts,
        front,
        shape,
        metrics,
        run,
    ))
}

/// The facts the record's link states, with the engine's own answer about
/// whether the job finished.
fn facts(root: &Path, record: &RedactedSession, store: &SqliteStateStore) -> Facts {
    let Some(link) = record.run.as_ref() else {
        return Facts::default();
    };
    // A job the store keeps no row for has no run to continue: it reads as
    // finished, and the resume states its own refusal rather than this front
    // inventing one.
    let finished = match block_on(store.get_coding_job(&link.job)) {
        Ok(Some(job)) => matches!(
            job.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ),
        Ok(None) | Err(_) => true,
    };
    resume::facts(root, &link.job, finished)
}

/// The engine's own resume of the linked job: the operation `saya job resume`
/// runs, under the session's own consent — the session's mode was the consent
/// to open, and it is the consent to continue.
async fn resume_job(
    application: &Arc<CodingApplication>,
    job: &JobId,
) -> Result<Arc<NativeCodingJob>, String> {
    let proposal = application
        .propose_resume(job)
        .await
        .map_err(|error| error.to_string())?;
    let consent = CodingAuthorization::scoped(&proposal).map_err(|error| error.to_string())?;
    application
        .resume(job, &consent, CancellationToken::new())
        .await
        .map_err(|error| error.to_string())
}
