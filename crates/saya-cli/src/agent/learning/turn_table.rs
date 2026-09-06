//! Turn-scoped object table and identifier mapping.
//!
//! Provides `TurnObjectId` (`T0..Tn`) and `TurnObjectTable` to map turn-scoped
//! identifiers to concrete database objects (Safety Property 2).

use serde::{Deserialize, Serialize};

/// At most 10 distinct database objects are tracked per turn record.
#[allow(dead_code)]
pub const MAX_TURN_OBJECTS: usize = 10;

/// A turn-scoped object identifier (`T0`, `T1`,...) presented to the extractor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TurnObjectId(String);

impl TurnObjectId {
    /// Creates a new `TurnObjectId` from a numeric index.
    #[allow(dead_code)]
    pub fn new(index: usize) -> Self {
        Self(format!("T{index}"))
    }

    /// Parses a string into a `TurnObjectId` if it matches `T\d+` (case-insensitive).
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.len() < 2 {
            return None;
        }
        let first = trimmed.chars().next()?;
        if first != 'T' && first != 't' {
            return None;
        }
        let rest = &trimmed[1..];
        if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
            let num: usize = rest.parse().ok()?;
            Some(Self(format!("T{num}")))
        } else {
            None
        }
    }

    /// Returns the string representation (e.g. `"T0"`).
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the underlying numeric index.
    #[allow(dead_code)]
    pub fn index(&self) -> usize {
        self.0[1..].parse::<usize>().unwrap_or(0)
    }
}

impl std::fmt::Display for TurnObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An entry in the turn-scoped object table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TurnObjectEntry {
    pub id: TurnObjectId,
    pub profile: String,
    pub qualified_name: String,
    pub columns: Vec<String>,
}

/// A bidirectional mapping between turn-scoped object IDs (`T0..Tn`) and concrete database objects.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TurnObjectTable {
    entries: Vec<TurnObjectEntry>,
}

impl TurnObjectTable {
    /// Creates an empty table.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn entries(&self) -> &[TurnObjectEntry] {
        &self.entries
    }

    /// Looks up an entry by its turn-scoped ID.
    #[allow(dead_code)]
    pub fn get_by_id(&self, id: &TurnObjectId) -> Option<&TurnObjectEntry> {
        self.entries.iter().find(|e| &e.id == id)
    }

    /// Looks up an entry by profile and qualified object name (case-insensitive).
    #[allow(dead_code)]
    pub fn get_by_name(&self, profile: &str, qualified_name: &str) -> Option<&TurnObjectEntry> {
        self.entries.iter().find(|e| {
            e.profile.eq_ignore_ascii_case(profile)
                && e.qualified_name.eq_ignore_ascii_case(qualified_name)
        })
    }

    /// Registers an object. If already present, merges new columns.
    /// If absent and within capacity, assigns the next `TurnObjectId`.
    #[allow(dead_code)]
    pub fn register(
        &mut self,
        profile: &str,
        qualified_name: &str,
        columns: &[String],
    ) -> Option<TurnObjectId> {
        if let Some(existing) = self.entries.iter_mut().find(|e| {
            e.profile.eq_ignore_ascii_case(profile)
                && e.qualified_name.eq_ignore_ascii_case(qualified_name)
        }) {
            for col in columns {
                if !existing.columns.iter().any(|c| c.eq_ignore_ascii_case(col)) {
                    existing.columns.push(col.clone());
                }
            }
            return Some(existing.id.clone());
        }

        if self.entries.len() >= MAX_TURN_OBJECTS {
            return None;
        }

        let id = TurnObjectId::new(self.entries.len());
        let mut deduped_columns: Vec<String> = Vec::new();
        for col in columns {
            if !deduped_columns.iter().any(|c| c.eq_ignore_ascii_case(col)) {
                deduped_columns.push(col.clone());
            }
        }

        self.entries.push(TurnObjectEntry {
            id: id.clone(),
            profile: profile.to_string(),
            qualified_name: qualified_name.to_string(),
            columns: deduped_columns,
        });

        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_object_table_assigns_monotonic_ids() {
        let mut table = TurnObjectTable::new();
        let id0 = table.register("primary", "catalog.public.orders", &["id".into()]);
        let id1 = table.register("primary", "catalog.public.users", &["user_id".into()]);

        assert_eq!(id0, Some(TurnObjectId::new(0)));
        assert_eq!(id1, Some(TurnObjectId::new(1)));
        assert_eq!(table.len(), 2);
        assert_eq!(id0.as_ref().unwrap().as_str(), "T0");
        assert_eq!(id0.as_ref().unwrap().index(), 0);
        assert_eq!(table.entries().len(), 2);

        let entry0 = table.get_by_id(&TurnObjectId::new(0)).unwrap();
        assert_eq!(entry0.qualified_name, "catalog.public.orders");
        assert_eq!(entry0.columns, vec!["id".to_string()]);

        let entry_by_name = table.get_by_name("primary", "catalog.public.orders");
        assert!(entry_by_name.is_some());
        assert_eq!(entry_by_name.unwrap().id, TurnObjectId::new(0));

        let id0_again = table.register("primary", "catalog.public.orders", &["created_at".into()]);
        assert_eq!(id0_again, Some(TurnObjectId::new(0)));
        assert_eq!(table.len(), 2);
        let entry0_updated = table.get_by_id(&TurnObjectId::new(0)).unwrap();
        assert_eq!(
            entry0_updated.columns,
            vec!["id".to_string(), "created_at".to_string()]
        );
    }

    #[test]
    fn test_turn_object_table_caps_at_max_objects() {
        let mut table = TurnObjectTable::new();
        for i in 0..15 {
            let id = table.register("primary", &format!("catalog.public.table_{i}"), &[]);
            if i < MAX_TURN_OBJECTS {
                assert_eq!(id, Some(TurnObjectId::new(i)));
            } else {
                assert_eq!(id, None, "Registration past MAX_TURN_OBJECTS returns None");
            }
        }
        assert_eq!(table.len(), MAX_TURN_OBJECTS);
    }

    #[test]
    fn test_turn_object_id_parsing() {
        assert_eq!(TurnObjectId::parse("T0"), Some(TurnObjectId::new(0)));
        assert_eq!(TurnObjectId::parse("t42"), Some(TurnObjectId::new(42)));
        assert_eq!(TurnObjectId::parse("T"), None);
        assert_eq!(TurnObjectId::parse("table0"), None);
        assert_eq!(TurnObjectId::parse("T12a"), None);
    }
}
