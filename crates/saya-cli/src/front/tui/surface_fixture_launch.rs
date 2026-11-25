//! The launch a scripted test runs: a real session over a throwaway folder,
//! against a scripted provider, opened by the launch's own opener
//! (`open_surface`) and wired to the port its commands are performed on.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use saya_agent::ApprovalPolicy;
use saya_store::{FsSessionStore, SqliteStateStore};
use tokio::sync::mpsc;

use super::direct_run::DirectDeps;
use super::direct_trust::TrustStore;
use super::jobs_open::JobDeps;
use super::line_surface::SurfaceLine;
use super::live::LiveRuntime;
use super::msg::Msg;
use super::ops::OpsParts;
use super::ops_deps::OpsDeps;
use super::run_cmd_port::{PortParts, SessionPort};
use super::run_front::Forward;
use super::surface_fixture::{Fixture, MODEL, RootGuard};
use crate::GlobalOptions;
use crate::chat::ChatOps;
use crate::commands::coding::parent_turn_provider::Scripted;
use crate::interactive::SessionState;
use crate::interactive::session::line_chat::open_surface;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::turn_loop::surface_parts;
use crate::load_with_sources;
use crate::render::RenderFormat;
use crate::slash::user_commands::UserCommands;

/// The profile a database fixture configures.
pub(crate) const DATABASE: &str = "demo";

/// The profile a database fixture states as the configured default, so the
/// profile `/connect` names is configured but not the one the run opens on.
const DEFAULT_PROFILE: &str = "other";

/// Whether the fixture configures a database profile, and whether the session
/// starts on it: what `/connect` moves, and what `/privacy` changes once the
/// family is on.
#[derive(Clone, Copy)]
enum Profile {
    None,
    /// Two profiles, the default one: the session `/connect` starts on.
    Configured,
    /// Two profiles, the fixture's database selected: a session whose run
    /// starts connected.
    Selected,
}

/// A surface over a throwaway folder, answering from a scripted provider.
pub(crate) async fn surface(label: &str) -> Fixture {
    surface_with(label, Profile::None).await
}

/// A surface with a profile configured but not selected: the run the session
/// starts has no database, and `/connect` is what binds one.
pub(crate) async fn surface_profile(label: &str) -> Fixture {
    surface_with(label, Profile::Configured).await
}

/// A surface whose session starts on the configured profile: its run is
/// connected from the first turn, so `/privacy` is observable on the tools a
/// request advertises once the family is on.
pub(crate) async fn surface_connected(label: &str) -> Fixture {
    surface_with(label, Profile::Selected).await
}

/// The throwaway database a database fixture's profile points at: a real,
/// empty SQLite file, so the profile opens rather than being skipped.
async fn database(root: &Path) -> PathBuf {
    let path = root.join("data.sqlite3");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options)
        .await
        .expect("the fixture database opens");
    sqlx::query("CREATE TABLE fixture (id INTEGER)")
        .execute(&pool)
        .await
        .expect("the fixture schema writes");
    pool.close().await;
    path
}

/// A surface over a throwaway folder, answering from a scripted provider, with
/// the database profile `profile` states.
async fn surface_with(label: &str, profile: Profile) -> Fixture {
    let provider = Scripted::start();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-surface-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the fixture root creates");
    let root = root.canonicalize().expect("the fixture root resolves");
    let folder = root.join("repo");
    fs::create_dir(&folder).expect("the fixture folder creates");
    let programs = root.join("programs");
    fs::create_dir(&programs).expect("the fixture program directory creates");
    let config_home = root.join("config");
    fs::create_dir_all(&config_home).expect("the fixture config directory creates");
    // A database fixture states its profiles where the launch reads one: two of
    // them, so the default the run opens on is not the one `/connect` names —
    // the runtime resolves a lone profile as the default, which would leave a
    // `/connect` nothing to move.
    let options = match profile {
        Profile::None => GlobalOptions::default(),
        Profile::Configured | Profile::Selected => {
            let database = database(&root).await;
            let connections = config_home.join("connections.toml");
            let entry = |name: &str| {
                format!(
                    "[profiles.{name}]\ntype = 'sqlite'\npath = '{}'\n",
                    database.display()
                )
            };
            fs::write(
                &connections,
                format!("{}{}", entry(DATABASE), entry(DEFAULT_PROFILE)),
            )
            .expect("the fixture connections file writes");
            GlobalOptions {
                connections: Some(connections),
                profile: Some(DEFAULT_PROFILE.to_owned()),
                ..GlobalOptions::default()
            }
        }
    };
    let mut runtime = load_with_sources(&options, &folder, &config_home, BTreeMap::new())
        .expect("the fixture resolves a runtime");
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.model = MODEL.into();
    runtime.resolved.ai.base_url = Some(provider.endpoint.clone());
    runtime.resolved.ai.api_key = None;
    runtime.resolved.ai.retry_delays_ms.clear();
    runtime.resolved.ai.timeout_seconds = 10;
    runtime.resolved.ai.idle_timeout_seconds = 10;
    runtime.resolved.ai.max_output_tokens = Some(4096);
    runtime.resolved.ai.context_window_tokens = Some(128_000);
    let effective = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .expect("the runtime resolves an orchestrator endpoint");
    // The endpoint an orchestrating turn reaches: the same scripted server, and
    // no model of its own — the session's own model is the one a request names,
    // which is what a `/model` change moves.
    effective.provider = saya_config::AiProvider::OpenaiCompatible;
    effective.base_url = Some(provider.endpoint.clone());
    effective.api_key = None;
    let runtime = Arc::new(runtime);
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
    // One session id per fixture: the job's own name, and the record's.
    let id = format!("surface-{label}");
    let session = SessionRuntime::acquire(
        &runtime,
        Some(&folder),
        true,
        None,
        &id,
        ApprovalPolicy::Ask,
        &root.join("session-dirs"),
    )
    .expect("the fixture's session acquires");
    // The session's own state, on the provider the scripted endpoint serves: a
    // state change re-resolves the provider and the endpoint through it, so the
    // two must agree with the runtime's own composition.
    let mut opening = SessionState::new(&id, None, MODEL);
    opening.provider = "openai_compatible".to_owned();
    opening.provider_endpoint = Some(provider.endpoint.clone());
    opening.provider_endpoint_bound = true;
    // A session that starts connected states the profile its run is composed
    // from, so the run the launch opens holds a database from its first turn.
    if matches!(profile, Profile::Selected) {
        opening.profile = Some(DATABASE.to_owned());
    }
    let state = Arc::new(Mutex::new(opening));
    let live = Arc::new(LiveRuntime::new(Arc::clone(&runtime)));
    let mut composed = surface_parts(
        &state.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        &session,
        &runtime,
        &state_db,
        RenderFormat::Text,
    );
    // The job's records go to this fixture's own root, never the operator's.
    composed.parts.state_root = Some(root.join("coding"));
    let (msgs, queue) = mpsc::unbounded_channel::<Msg>();
    let front = Arc::new(Forward::new(msgs));
    let opened = open_surface(
        composed.parts,
        &composed.record,
        Arc::clone(&front) as Arc<dyn crate::chat::ChatFront>,
    )
    .await
    .expect("the launch's run opens");
    let line = Arc::new(SurfaceLine::new(opened.into_line(), Arc::clone(&front)));
    let deps = Arc::new(OpsDeps::new(
        ChatOps::new(
            line.run_port().expect("the launch's run has a port"),
            line.context(true)
                .expect("the launch's run has a mode cell"),
        ),
        UserCommands::default(),
        Arc::clone(&live),
        Arc::clone(&state),
        OpsParts {
            store: FsSessionStore::new(root.join("session-store")),
            state_db: state_db.clone(),
            format: RenderFormat::Text,
            root: Some(folder.clone()),
            policy: session.policy(),
            journal: session.journal(),
            universe: session.universe(),
            recovery: session.recovery().clone(),
        },
    ));
    let jobs = Arc::new(JobDeps::new(
        Arc::clone(&live),
        state_db.clone(),
        Some(&folder),
        &id,
        session.policy().mode(),
    ));
    let direct = Arc::new(DirectDeps {
        runtime: Arc::clone(&live),
        state_db,
        format: RenderFormat::Text,
        trust: Arc::new(NoTrust) as Arc<dyn TrustStore>,
    });
    let port = Arc::new(SessionPort::new(PortParts {
        line: Arc::clone(&line),
        front: Arc::clone(&front),
        allow_bypass: true,
        direct,
        deps,
        options: GlobalOptions::default(),
        runtime: live,
        state: Arc::clone(&state),
        jobs,
        state_root: Some(root.join("coding")),
    }));
    let fixture = Fixture {
        words: Arc::new(Mutex::new(Vec::new())),
        picked: Arc::new(Mutex::new(None)),
        folder,
        front: Arc::clone(&front),
        port,
        provider,
        state,
        line,
        _session: session,
        _root: RootGuard(root),
    };
    fixture.drain(queue);
    fixture
}

/// A trust store that binds nothing: these tests ask no trust question.
struct NoTrust;

impl TrustStore for NoTrust {
    fn bind(&self, _dir: &Path) -> Result<(), String> {
        Ok(())
    }
}
