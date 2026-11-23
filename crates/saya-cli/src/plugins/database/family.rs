//! When this run advertises the database family: the latch that keeps it on for
//! the session, the bound connections it is about, and the one tiny tool a model
//! can call to ask for it. The evidence a request has to show lives in
//! [`super::evidence`].
//!
//! A bound profile used to inject the database tools and the database section of
//! the system prompt into every turn of a session, whatever the request was
//! about — roughly half of a chat's system prompt, paid on a request that may
//! never touch a database. So the family is advertised from evidence in the
//! request and latched once it appears, as the web tools and `helpers` are
//! (`agent::tools::advertisement`): a latch, not a per-turn filter, so the
//! request prefix a provider caches moves at most once and never after.
//!
//! A coding job and a helper say "on from the start" instead: their database
//! scope was narrowed by their own consent before the run existed, so they
//! compose [`Family::always`] and see exactly what they saw before this rule.

use std::sync::atomic::{AtomicBool, Ordering};

use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use saya_types::SqlDialect;

use super::evidence::shows_work;
use crate::connection::ConnectionRegistry;

/// The one name advertised before the family is latched: the ask a model makes
/// when the request itself never said "database". It stays in the list once the
/// family is on, at the same position, so the latch only ever appends.
pub(crate) const USE_DATABASE: &str = "use_database";

/// The connections this run holds, with the object names it has cached from
/// their schema. Both are evidence a request can name without saying
/// "database": "what does `orders` hold" is database work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Bound {
    connections: Vec<(String, SqlDialect)>,
    objects: Vec<String>,
}

impl Bound {
    pub(crate) fn new(connections: Vec<(String, SqlDialect)>, objects: Vec<String>) -> Self {
        Self {
            connections,
            objects,
        }
    }

    /// The run's own connection set, primary first, with no cached objects.
    pub(crate) fn of(registry: &ConnectionRegistry) -> Self {
        Self {
            connections: registry
                .entries()
                .into_iter()
                .map(|(name, entry)| (name.to_owned(), entry.dialect))
                .collect(),
            objects: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }

    /// The same connections, with the object names the run's schema cache
    /// holds: a request that names one is database work.
    pub(crate) fn with_objects(mut self, objects: Vec<String>) -> Self {
        self.objects = objects;
        self
    }

    /// The connections and their engines in one line: `local (sqlite), warehouse
    /// (snowflake)`.
    pub(crate) fn one_line(&self) -> String {
        self.connections
            .iter()
            .map(|(name, dialect)| format!("{name} ({})", dialect.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The names a request may state instead of saying "database": the
    /// connections' own names, the engines they speak, and the object names
    /// cached from their schema — lowercased, because the request is compared
    /// lowercased.
    pub(super) fn evidence_names(&self) -> Vec<String> {
        self.connections
            .iter()
            .map(|(name, dialect)| [name.as_str(), dialect.as_str()])
            .chain(
                self.objects
                    .iter()
                    .map(|object| [object.as_str(), object.as_str()]),
            )
            .flat_map(|pair| pair.map(str::to_ascii_lowercase))
            .collect()
    }
}

/// Whether the run's one `[tools] allow` list names a database tool: the
/// explicit override, the door the web tools and `helpers` use too. The list
/// cannot take one of these names away (they follow the run's connection), so
/// naming one is a request for the family, never a narrowing.
pub(crate) fn stated_by(tools: &saya_config::ResolvedTools) -> bool {
    super::DECLARED_TOOLS.iter().any(|name| tools.states(name))
}

/// The family's latch and the evidence it is turned on by.
pub(crate) struct Family {
    bound: Bound,
    /// The run's one `[tools] allow` list names a database tool: the explicit
    /// override, on from the first turn like the web tools' and `helpers`'.
    stated: bool,
    /// A surface that sees the family from its first turn: a coding job or a
    /// helper, whose own consent already narrowed the database scope.
    always: bool,
    latched: AtomicBool,
}

impl Family {
    /// The evidence rule, over a run whose connections are `bound`.
    pub(crate) fn of(bound: Bound, stated: bool) -> Self {
        Self {
            bound,
            stated,
            always: false,
            latched: AtomicBool::new(false),
        }
    }

    /// The family on from the first turn, with nothing to announce: a job or a
    /// helper surface sees the list it always saw.
    pub(crate) fn always() -> Self {
        Self {
            bound: Bound::default(),
            stated: false,
            always: true,
            latched: AtomicBool::new(false),
        }
    }

    /// Turns the family on for the rest of the session.
    pub(crate) fn latch(&self) {
        self.latched.store(true, Ordering::Release);
    }

    /// Whether the family is advertised now.
    pub(crate) fn latched(&self) -> bool {
        self.always || self.stated || self.latched.load(Ordering::Acquire)
    }

    /// Whether `text` shows database work, latching when it does and reporting
    /// whether the family is on afterwards.
    pub(crate) fn note(&self, text: &str) -> bool {
        if self.latched() {
            return true;
        }
        if shows_work(text, &self.bound) {
            self.latch();
            return true;
        }
        false
    }

    /// The one tool advertised while the family is off, or `None` when the run
    /// holds no connection.
    pub(crate) fn announces(&self) -> Option<ToolDefinition> {
        if self.bound.is_empty() {
            return None;
        }
        Some(Self::definition(format!(
            "Turn on the database tools for the bound connection(s): {}.",
            self.bound.one_line()
        )))
    }

    /// What a `use_database` call answers: the connections and their engines in
    /// one line, and the fact that the tools are now advertised.
    pub(crate) fn result(&self) -> serde_json::Value {
        serde_json::json!({
            "connections": self.bound.one_line(),
            "tools": "on",
        })
    }

    /// A definition shaped like `use_database`, for the name catalogs.
    pub(crate) fn definition(description: String) -> ToolDefinition {
        ToolDefinition {
            name: USE_DATABASE.into(),
            description,
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }
    }
}
