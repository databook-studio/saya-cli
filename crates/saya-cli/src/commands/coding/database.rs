//! The coding job's data surface: the standard read-only database tools
//! (`schema_discovery`, `bounded_sql_query`) over the job's admitted profiles,
//! and the clarification ask, which is not a database tool at all.
//!
//! The coding root and its helpers advertise the SAME tools the chat agent
//! does — the definitions and the executor are `DatabaseTools`', never a copy —
//! narrowed to two things: the profiles this job's consent selected, and the
//! read-only tools. Exact profile admission is re-checked per call against the
//! consent-bound identity, and only the requested profile is ever connected.
//! The ask sits on the same surface because it rides the same routing, but its
//! two gates are the other way round: it needs no connection, so it is
//! advertised whenever an operator surface can answer it — even when the job
//! selected no profile at all — and never when none can.
use super::{CodingApplication, CodingError, CodingJobRequest};
use crate::agent::tools::DatabaseTools;
use crate::agent::tools::clarification;
use crate::config::runtime::RuntimeConfig;
use saya_agent::{ToolDefinition, ToolError, ToolExecutionContext, ToolExecutor};
use saya_harness::coding::policy::ChildAuthorityView;
use saya_store::SqliteStateStore;
use saya_types::ProfileIdentity;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The standard database tools the coding surfaces advertise. Everything else
/// the chat toolset carries (the chart, contract and fan-out tools) stays off
/// the coding surface: the coding root and its helpers read a schema and run
/// one bounded read-only query.
const CODING_DATABASE_TOOLS: [&str; 2] = ["schema_discovery", "bounded_sql_query"];

/// Whether `name` is one of the standard database tools the coding surfaces
/// advertise. One list decides both what is advertised and what is routed.
pub(crate) fn is_database_tool(name: &str) -> bool {
    CODING_DATABASE_TOOLS.contains(&name)
}

/// Whether `name` is served by this surface: the standard database tools, or
/// the ask. One predicate decides both what is routed here and — with the two
/// separate gates inside — what is advertised.
pub(crate) fn routes_here(name: &str) -> bool {
    is_database_tool(name) || clarification::is_ask(name)
}

pub(crate) struct CodingDatabaseTools {
    runtime: RuntimeConfig,
    store: SqliteStateStore,
    selected: BTreeMap<String, ProfileIdentity>,
    /// Whether an operator surface is attached to answer the clarification
    /// ask. The SQL tools never depend on it; the ask never depends on a
    /// profile.
    clarification: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    connection: String,
    sql: Option<String>,
}

impl CodingDatabaseTools {
    pub(crate) fn new(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        selected: BTreeMap<String, ProfileIdentity>,
        clarification: bool,
    ) -> Self {
        Self {
            runtime,
            store,
            selected,
            clarification,
        }
    }

    /// The definitions the coding root and its helpers advertise: the
    /// clarification ask when an operator can answer it, plus the standard
    /// read-only database tools narrowed to the profiles this view admits. The
    /// SQL half is empty when this view admits no profile, so a job with no
    /// selected profile advertises no way to reach a database; the ask is
    /// advertised whether or not one is selected.
    pub(crate) fn definitions(&self, view: Option<&ChildAuthorityView>) -> Vec<ToolDefinition> {
        let mut definitions = Vec::new();
        if self.advertises_clarification() {
            definitions.push(clarification::definition());
        }
        let profiles: Vec<&str> = self
            .selected
            .iter()
            .filter(|(_, identity)| {
                view.is_none_or(|view| view.admit_database_profile(identity).is_ok())
            })
            .map(|(name, _)| name.as_str())
            .collect();
        if profiles.is_empty() {
            return definitions;
        }
        let mut queries = DatabaseTools::definitions(
            self.runtime.resolved.ai.allow_data_sharing,
            false,
            false,
            false,
        );
        queries.retain(|definition| CODING_DATABASE_TOOLS.contains(&definition.name.as_str()));
        for definition in &mut queries {
            narrow_to_profiles(definition, &profiles);
        }
        definitions.extend(queries);
        definitions
    }

    /// Whether this surface advertises the ask: only when an operator surface
    /// is attached to answer it.
    pub(crate) fn advertises_clarification(&self) -> bool {
        self.clarification
    }

    /// The ask's fixed result, when this surface advertises it. `None` when it
    /// does not: a name the surface never advertised is unknown to it, so a
    /// job with no operator can never be made to stop for a question nobody
    /// can answer.
    pub(crate) fn ask_result(&self) -> Option<Value> {
        self.advertises_clarification().then(clarification::result)
    }

    /// Runs one standard database tool call against exactly the profile it
    /// names, after re-checking that the profile is one this job's consent
    /// selected, that the view still admits it, and that its identity has not
    /// changed since consent. The ask resolves first and touches none of it:
    /// no connection is ever built for it.
    pub(crate) async fn execute(
        &self,
        view: Option<&ChildAuthorityView>,
        name: &str,
        arguments: Value,
        context: ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        if clarification::is_ask(name) {
            return self.ask_result().ok_or(ToolError::UnsupportedTool);
        }
        let args: Arguments =
            serde_json::from_value(arguments).map_err(|_| refused("invalid database arguments"))?;
        let identity = self
            .selected
            .get(&args.connection)
            .ok_or_else(|| refused("database profile not approved"))?;
        if let Some(view) = view {
            view.admit_database_profile(identity)
                .map_err(|_| refused("database profile not approved"))?;
        }
        let profile = self
            .runtime
            .connections
            .profiles
            .get(&args.connection)
            .ok_or_else(|| refused("database profile unavailable"))?;
        if crate::profile_identity::profile_identity(
            &args.connection,
            profile,
            &self.runtime.cache_scope,
        ) != *identity
        {
            return Err(refused(
                "database profile identity changed; start a newly approved job",
            ));
        }
        let resolver = self.runtime.secret_resolver();
        // Only this admitted profile is ever connected. No configured secondary is probed.
        let connection = crate::connection::build_registry(
            (&resolver, &self.runtime.cache_scope),
            self.runtime.resolved.query_timeout_seconds,
            false,
            &args.connection,
            profile,
            &[],
            None,
        );
        // Some factory stages (notably SQLite open) precede connector.connect.
        // Reuse the query policy for an overall build/connect deadline too.
        let deadline =
            std::time::Duration::from_secs(self.runtime.resolved.query_timeout_seconds.max(1));
        let (registry, _) = tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => return Err(refused("database request cancelled")),
            result = tokio::time::timeout(deadline, connection) => result
                .map_err(|_| refused("database connection timed out"))?
                .map_err(|error| ToolError::Runner(error.to_string()))?,
        };
        if registry.name_for_identity(identity.as_str()) != Some(args.connection.as_str()) {
            return Err(refused(
                "connected database identity differs from approved identity",
            ));
        }
        // The standard toolset, over the one connection this call admitted.
        let tools = DatabaseTools::with_learning(
            registry,
            self.runtime.resolved.max_rows,
            self.runtime.resolved.ai.allow_data_sharing,
            Some(self.store.clone()),
            None,
        );
        // `sql: null` is not a property the standard tools accept, so the
        // canonical arguments are rebuilt from the parsed shape.
        let mut canonical = json!({"connection": args.connection});
        if let Some(sql) = args.sql {
            canonical["sql"] = json!(sql);
        }
        tools.execute_with_context(name, canonical, context).await
    }
}

/// Narrows one standard definition to this job's admitted profiles: the
/// `connection` property enumerates exactly them and is required, so the model
/// is told which profiles exist instead of discovering it from a refusal.
fn narrow_to_profiles(definition: &mut ToolDefinition, profiles: &[&str]) {
    let Some(properties) = definition
        .parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let Some(connection) = properties
        .get_mut("connection")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    connection.insert("description".into(), json!(PROFILE_DESCRIPTION));
    connection.insert("maxLength".into(), json!(128));
    connection.insert("enum".into(), json!(profiles));
    definition.parameters["required"] = if definition.name == "bounded_sql_query" {
        json!(["connection", "sql"])
    } else {
        json!(["connection"])
    };
}

const PROFILE_DESCRIPTION: &str = "The database profile to target: one of the profiles selected \
for this job.";

fn refused(message: &str) -> ToolError {
    ToolError::NoEffect(Box::new(ToolError::Runner(message.into())))
}

impl super::NativeCodingJob {
    /// The job's data surface, or `None` when it would advertise nothing: the
    /// job selected no profile and no operator surface is attached to answer
    /// the ask.
    pub(crate) fn databases(&self) -> Option<CodingDatabaseTools> {
        (self.operator_present() || !self.request.database_identities.is_empty()).then(|| {
            CodingDatabaseTools::new(
                self.application.runtime.clone(),
                self.application.store.clone(),
                self.request.database_identities.clone(),
                self.operator_present(),
            )
        })
    }
}

/// Which profiles this job may use, and the local-path binding that runs
/// before consent. Sibling concern: no connection exists in either.
#[path = "database/selection.rs"]
mod selection;
pub(in crate::commands::coding) use selection::bind_local_profiles;
