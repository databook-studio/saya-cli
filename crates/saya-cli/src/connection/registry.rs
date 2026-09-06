use saya_agent::ToolError;
use saya_connectors::DatabaseConnector;
use saya_types::SqlDialect;
use std::collections::HashMap;
use std::fmt;

/// Represents a live database connection entry.
#[allow(dead_code)]
pub(crate) struct ConnectionEntry {
    /// The database connector implementation.
    pub(crate) connector: Box<dyn DatabaseConnector>,
    /// The SQL dialect of the connection.
    pub(crate) dialect: SqlDialect,
    /// Optional database profile identifier.
    pub(crate) profile_id: Option<String>,
}

impl fmt::Debug for ConnectionEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionEntry")
            .field("dialect", &self.dialect)
            .field("profile_id", &self.profile_id)
            .finish()
    }
}

/// A typed registry of live database connections for multi-database agent navigation.
#[allow(dead_code)]
pub(crate) struct ConnectionRegistry {
    primary: String,
    names: Vec<String>,
    map: HashMap<String, ConnectionEntry>,
}

#[allow(dead_code)]
impl ConnectionRegistry {
    /// Creates an empty registry whose primary connection is `primary`.
    pub(crate) fn new(primary: impl Into<String>) -> Self {
        Self {
            primary: primary.into(),
            names: Vec::new(),
            map: HashMap::new(),
        }
    }

    /// Inserts or replaces a named connection, preserving first-seen order.
    pub(crate) fn insert(&mut self, name: impl Into<String>, entry: ConnectionEntry) {
        let name = name.into();
        if !self.map.contains_key(&name) {
            self.names.push(name.clone());
        }
        self.map.insert(name, entry);
    }

    /// Returns the primary connection name.
    pub(crate) fn primary_name(&self) -> &str {
        &self.primary
    }

    /// Returns the number of connections in the registry.
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns true if the registry contains no connections.
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Connection names in insertion order.
    pub(crate) fn names(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }

    /// Every connection with its name, in insertion order. This is the
    /// currently included/connected set that multi-database fan-out runs
    /// against (primary plus each secondary that connected).
    pub(crate) fn entries(&self) -> Vec<(&str, &ConnectionEntry)> {
        self.names
            .iter()
            .filter_map(|name| self.map.get(name).map(|entry| (name.as_str(), entry)))
            .collect()
    }

    /// Resolves an optional connection name to an entry. `None` or empty -> primary.
    /// Unknown name -> Err with a message listing the available names.
    /// Empty registry -> Err("no database profile is selected").
    pub(crate) fn resolve(&self, name: Option<&str>) -> Result<&ConnectionEntry, ToolError> {
        let target = match name {
            None | Some("") => self.primary.as_str(),
            Some(n) => n,
        };
        if let Some(entry) = self.map.get(target) {
            Ok(entry)
        } else if self.is_empty() {
            Err(ToolError::NoConnectionSelected)
        } else {
            let available = self.names().join(", ");
            Err(ToolError::UnknownConnection {
                target: target.to_string(),
                available,
            })
        }
    }

    /// The connection name whose stored profile identity is `identity`, if any.
    ///
    /// The observation collector records the opaque [`ProfileIdentity`] (the
    /// stable object identity, never a name a user chose); the turn record
    /// resolves objects back to a connection *by name*, because that is what
    /// [`ConnectionRegistry::resolve`] keys on. This is the one place the
    /// identity the observation carries is turned into the name the resolver
    /// expects, so a turn that touched a non-primary connection is attributed to
    /// the connection it actually used rather than collapsed onto the primary.
    pub(crate) fn name_for_identity(&self, identity: &str) -> Option<&str> {
        self.entries()
            .into_iter()
            .find(|(_, entry)| entry.profile_id.as_deref() == Some(identity))
            .map(|(name, _)| name)
    }

    /// The dialect of every connection, in registration order.
    ///
    /// Callers that must phrase something per engine — SQL naming rules, for
    /// one — need this even when `describe_context` stays silent because there
    /// is only a single connection.
    pub(crate) fn dialects(&self) -> impl Iterator<Item = SqlDialect> + '_ {
        self.names
            .iter()
            .filter_map(|name| self.map.get(name).map(|entry| entry.dialect))
    }

    /// System-prompt addendum listing every connection and its dialect, instructing the
    /// model to pass the `connection` argument and inspect each database separately then
    /// combine findings. Returns None when there is <= 1 connection (no navigation needed).
    pub(crate) fn describe_context(&self) -> Option<String> {
        if self.len() <= 1 {
            return None;
        }

        let mut lines = Vec::new();
        lines.push("Available database connections:".to_string());
        for name in &self.names {
            if let Some(entry) = self.map.get(name) {
                lines.push(format!("- {name} ({})", entry.dialect.as_str()));
            }
        }
        lines.push(
            "To inspect a database, pass its `connection` argument to schema and query tools. Inspect each database separately and combine your findings. When the same query should run against every connected database, call `bounded_sql_query_all` once instead of repeating `bounded_sql_query` per connection."
                .to_string(),
        );
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
