//! What the run a slash command gives up leaves behind.
//!
//! The line loop composes the session's first run at the prompt (PERF1), and an
//! input a run is composed from — a slash command — gives that run up. The
//! folder was already leased, under this session's name, by the job the give-up
//! closes: afterwards the lease must be free and the job must be gone, so a
//! session that starts with a slash command leaves nothing working, no folder
//! held and no job of its own recorded.
#![cfg(all(test, unix))]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use saya_agent::{ApprovalPolicy, SessionGrants};
use saya_harness::coding::local::LocalRoot;
use saya_store::{RedactedSession, SqliteStateStore};
use saya_types::JobId;

use super::LineChat;
use super::open::LineSession;
use crate::GlobalOptions;
use crate::agent::turn_config::PromptOverrides;
use crate::commands::coding::{folder_lease_held, folder_lease_owner, job_records};
use crate::config::runtime::load_with_sources;
use crate::plugins::database::DatabaseBudget;
use crate::render::RenderFormat;

/// The session that states a preference before its first line.
const SESSION: &str = "session-prepared-close";

/// A throwaway base directory outside any repository, canonical and private,
/// removed when the case ends: the job's state root is owner-private and lives
/// outside the folder's repository.
fn case(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "saya-prepared-close-{label}-{}-{stamp}",
        process::id()
    ));
    for path in [base.clone(), base.join("site"), base.join("state")] {
        private_dir(&path);
    }
    base.canonicalize().unwrap()
}

/// An owner-private directory: the rule the host applies to every state root it
/// opens over.
fn private_dir(path: &Path) {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    use std::os::unix::fs::DirBuilderExt;
    builder.mode(0o700);
    builder.create(path).unwrap();
}

/// The session's facts over `base`'s folder, recording under `base`'s state
/// root: the provider is a local endpoint that is never asked, and the mode
/// reaches the write family, so the prepared run states a write scope and takes
/// the folder's lease.
fn session(base: &Path) -> LineSession {
    let folder = base.join("site");
    let records = base.join("records");
    private_dir(&records);
    let mut runtime = load_with_sources(
        &GlobalOptions::default(),
        &folder,
        &records,
        BTreeMap::new(),
    )
    .expect("the folder's runtime resolves");
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.model = "prepared-close".into();
    runtime.resolved.ai.base_url = Some("http://127.0.0.1:9".into());
    runtime.resolved.ai.api_key = None;
    runtime.resolved.ai.retry_delays_ms.clear();
    runtime.resolved.ai.timeout_seconds = 5;
    LineSession {
        session: SESSION.to_owned(),
        folder,
        runtime: Arc::new(runtime),
        store: SqliteStateStore::new(records.join("state.sqlite3")),
        mode: ApprovalPolicy::Bypass,
        overrides: PromptOverrides::default(),
        format: RenderFormat::Text,
        sink: None,
        budget: DatabaseBudget::defaults(),
        can_prompt: false,
        grants: SessionGrants::default(),
        state_root: Some(base.join("state")),
        cancellation: None,
        tasks: None,
        facts: None,
        seed: Vec::new(),
    }
}

/// The run the loop composes at the prompt takes the folder's lease, and ends
/// with it given back: the mechanism the give-up below relies on, read on the
/// composition itself.
#[test]
fn the_run_composed_at_the_prompt_leases_the_folder_until_it_is_ended() {
    let base = case("composed");
    let parts = session(&base);
    let state = base.join("state");
    let root = LocalRoot::open(&parts.folder, &state).expect("the folder is an in-place root");
    let root_id = root.id().as_str().to_owned();

    let chat = match LineChat::begin(parts.clone(), RedactedSession::default()) {
        Ok(super::Begun::Open { chat, .. }) => chat,
        Ok(super::Begun::Refused { sentence }) => panic!("the run is refused: {sentence}"),
        Err(error) => panic!("the run could not be composed: {error}"),
    };
    assert!(
        folder_lease_held(&state, &root_id).expect("the folder's lease is readable"),
        "the run composed at the prompt leases the folder"
    );

    chat.end_prepared_run();
    drop(chat);
    assert!(
        !folder_lease_held(&state, &root_id).expect("the folder's lease is readable"),
        "the run's close releases the folder"
    );
    let _ = fs::remove_dir_all(&base);
}

/// The run the loop composed at the prompt, given up the way a slash command
/// gives it up: the folder is free, and the job is gone from disk.
#[test]
fn a_given_up_prepared_run_releases_the_folder_its_lease_held() {
    let base = case("lease");
    let parts = session(&base);
    let state = base.join("state");
    let root = LocalRoot::open(&parts.folder, &state).expect("the folder is an in-place root");
    let root_id = root.id().as_str().to_owned();

    LineChat::prepare(parts.clone(), RedactedSession::default()).abandon();

    assert!(
        !folder_lease_held(&state, &root_id).expect("the folder's lease is readable"),
        "the abandoned run released the folder: no live owner holds its lease"
    );
    assert_eq!(
        folder_lease_owner(&root_id),
        None,
        "the abandoned run gave its lease back to the process's own registry"
    );
    let job = JobId::parse(&parts.session).expect("the session's name is its job's id");
    let facts = job_records::facts(&state, &job);
    assert!(
        !facts.records_present,
        "the abandoned run left no job of its own on disk"
    );
    let _ = fs::remove_dir_all(&base);
}
