//! The fixture harness the client's tests share: a configured server built the
//! way a user's configuration builds one, and the two fakes a consent test
//! needs.
//!
//! The servers come from a real `[mcp_servers]` table through the real resolve
//! path, not from a struct literal: the bounds this module's tests care about
//! are the ones resolution applies, and a hand-built value would skip them.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_config::{
    ConfigFile, ConnectionsFile, MapSecretResolver, ResolutionInput, ResolvedMcpServers,
};
use serde_json::Value;

use super::McpTools;
use super::fixture;
use crate::agent::tools::consent::{Caller, Operator};

/// The timeouts a fixture server is configured with. Generous on purpose: the
/// bound these tests are about is the client's own, and a loaded test host
/// starting a shell must not read as a server that failed to answer.
pub(crate) const TIMEOUTS: (u64, u64) = (20_000, 20_000);

/// One fixture server script, in its own directory.
pub(crate) struct Fixture {
    script: PathBuf,
}

impl Fixture {
    pub(crate) fn new(label: &str) -> Self {
        let dir = fixture::dir(label);
        let script = fixture::server_script(&dir);
        Self { script }
    }

    /// The resolved `[mcp_servers]` table for this fixture: one server named
    /// `fixture`, running the script, handed exactly `vars` from the resolver.
    pub(crate) fn servers(
        &self,
        vars: &[(&str, &str)],
        timeouts: (u64, u64),
    ) -> ResolvedMcpServers {
        let env = vars
            .iter()
            .map(|(name, _)| format!("{name} = {{ env = \"{name}\" }}"))
            .collect::<Vec<_>>()
            .join(", ");
        let toml = format!(
            "[mcp_servers.fixture]\ncommand = '{}'\nstartup_timeout_ms = {}\n\
             call_timeout_ms = {}\nenv = {{ {env} }}\n",
            self.script.display(),
            timeouts.0,
            timeouts.1,
        );
        let resolved = saya_config::resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(&toml).expect("the fixture table parses")),
        )
        .expect("the fixture table resolves");
        resolved.mcp_servers
    }

    pub(crate) fn resolver(&self, vars: &[(&str, &str)]) -> MapSecretResolver {
        MapSecretResolver::new(
            vars.iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        )
    }

    pub(crate) fn oversized_description(&self) {
        fixture::oversized_description(&self.script);
    }

    pub(crate) fn oversized_schema(&self) {
        fixture::oversized_schema(&self.script);
    }
}

/// One composed client, under `policy`, against the session's own caller.
pub(crate) fn lane(
    servers: &ResolvedMcpServers,
    resolver: &dyn saya_config::SecretResolver,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
) -> Option<Arc<McpTools>> {
    lane_with_cancellation(
        servers,
        resolver,
        policy,
        operator,
        CancellationToken::new(),
    )
}

/// The same client, with the cancellation token a test controls.
pub(crate) fn lane_with_cancellation(
    servers: &ResolvedMcpServers,
    resolver: &dyn saya_config::SecretResolver,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
    cancellation: CancellationToken,
) -> Option<Arc<McpTools>> {
    McpTools::connect(
        servers,
        resolver,
        policy,
        operator,
        &Caller::Session,
        cancellation,
    )
}

/// An operator that answers `approve` and remembers every line it was shown.
pub(crate) struct RecordingOperator {
    approve: bool,
    subjects: Mutex<Vec<String>>,
}

pub(crate) fn recording_operator(approve: bool) -> Arc<RecordingOperator> {
    Arc::new(RecordingOperator {
        approve,
        subjects: Mutex::new(Vec::new()),
    })
}

impl RecordingOperator {
    /// The line of every ask this operator answered, in order.
    pub(crate) fn subjects(&self) -> Vec<String> {
        self.subjects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl Operator for RecordingOperator {
    async fn ask(&self, subject: &str, _arguments: &Value) -> bool {
        self.subjects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(subject.to_owned());
        self.approve
    }
}
