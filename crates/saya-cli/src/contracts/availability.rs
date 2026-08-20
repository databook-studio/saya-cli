//! Schema availability: the three-state input to read-time validity.
//!
//! Recall and the contract read tools classify a stored claim against the
//! schema *known for its profile*. That knowledge has three genuinely
//! different shapes, and collapsing any two of them is the P1 bug this module
//! exists to fix:
//!
//! - [`SchemaAvailability::Available`] — a cached schema exists, with the
//!   instant it was observed. Validity compares the claim's stored fingerprint
//!   to the live table and can return `Current`/`NeedsReview`/`Stale`.
//! - [`SchemaAvailability::Missing`] — no cache entry for the profile. Nothing
//!   has been discovered yet. Honest answer: we cannot classify.
//! - [`SchemaAvailability::Unavailable`] — the store could not be read. Distinct
//!   from `Missing`: one is "nothing known", the other is "cannot find out". A
//!   diagnostic should be able to say which, even though both classify the
//!   same today.
//!
//! `Missing` and `Unavailable` both map to
//! [`crate::contracts::view::ContractSchemaState::LiveSchemaUnavailable`],
//! never to `Stale`. An unreadable or absent cache is not evidence that a
//! column is gone — the same rule `reconcile` already obeys (an unreachable
//! database is not evidence that anything changed).
//!
//! [`SchemaFreshness`] gates *currency* on the model path only: a cached schema
//! older than the bound must not classify a claim as `Current`, because we
//! cannot vouch for it past the bound. Human-review paths pass
//! [`SchemaFreshness::Unbounded`] — a reviewer looking at their own contracts
//! is not being asked to trust a query built on them.

use saya_types::SchemaTree;
use std::time::{SystemTime, UNIX_EPOCH};

/// The staleness bound for **model-facing** recall. A cached schema older than
/// this must not classify a claim as `Current` — "current" must mean "matches
/// the schema now", not "matches whatever we last wrote down". See the task
/// report's freshness argument for why 24h is the default and not a tuning
/// knob.
pub(crate) const MODEL_SCHEMA_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;

/// The wall clock the model path bounds cached-schema freshness against.
/// Centralised here so recall and the contract read tools compare against the
/// same "now"; tests pass a controlled stamp into the pure freshness check
/// instead, so the bound itself stays deterministic.
pub(crate) fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

/// What the schema layer knows about a profile's schema. See the module docs
/// for the three-way distinction and why collapsing any two is a bug.
#[derive(Debug, Clone)]
pub(crate) enum SchemaAvailability {
    /// A cached schema exists. `observed_at_unix_ms` is when the cache was
    /// written (the store's `updated_unix_ms`); a freshness check compares it
    /// to "now" on the model path.
    Available {
        schema: SchemaTree,
        observed_at_unix_ms: i64,
    },
    /// No cache entry for this profile — nothing has been discovered yet.
    Missing,
    /// The store could not be read. Distinct from `Missing`: one is "nothing
    /// known", the other is "cannot find out".
    Unavailable,
}

/// Whether a cached schema's age gates its use, and against what bound. The
/// model path bounds age (a stale-by-age cache cannot vouch for currency); the
/// human-review path does not (a reviewer is not asked to trust a query built
/// on their own contracts).
#[derive(Debug, Clone, Copy)]
pub(crate) enum SchemaFreshness {
    /// A schema older than `max_age_ms` before `now_unix_ms` is treated as
    /// unavailable for classifying a claim as `Current`.
    Bounded { now_unix_ms: i64, max_age_ms: i64 },
    /// The cached schema is used as-is regardless of age. Human-review paths.
    Unbounded,
}

impl SchemaFreshness {
    /// The model-facing default: bounded by [`MODEL_SCHEMA_MAX_AGE_MS`] against
    /// `now_unix_ms`. Recall and the contract read tools use this so a stale
    /// cache cannot assert a fact the system cannot support.
    pub(crate) fn for_model(now_unix_ms: i64) -> Self {
        Self::Bounded {
            now_unix_ms,
            max_age_ms: MODEL_SCHEMA_MAX_AGE_MS,
        }
    }
}

impl SchemaAvailability {
    /// Convenience for tests and live-schema call sites that have just
    /// fetched a schema: it is current as of `observed_at_unix_ms`. Production
    /// callers pass the store's `updated_unix_ms`; tests pass a controlled
    /// stamp so the freshness comparison stays deterministic.
    pub(crate) fn available(schema: SchemaTree, observed_at_unix_ms: i64) -> Self {
        Self::Available {
            schema,
            observed_at_unix_ms,
        }
    }

    /// The schema tree to classify against, but only when the cache is
    /// [`SchemaAvailability::Available`] **and** (when freshness is bounded)
    /// within the age bound. `Missing`, `Unavailable`, and a too-old `Available`
    /// all return `None`: the caller must classify `LiveSchemaUnavailable`,
    /// never `Stale` or `Current`.
    pub(crate) fn live_table_schema(&self, freshness: SchemaFreshness) -> Option<&SchemaTree> {
        let Self::Available {
            schema,
            observed_at_unix_ms,
        } = self
        else {
            return None;
        };
        match freshness {
            SchemaFreshness::Unbounded => Some(schema),
            SchemaFreshness::Bounded {
                now_unix_ms,
                max_age_ms,
            } => {
                // A cache whose age exceeds the bound (or whose clock compare
                // underflows) cannot vouch for currency. "Older than" is
                // strict: a cache exactly `max_age` old is still fresh.
                if now_unix_ms.saturating_sub(*observed_at_unix_ms) <= max_age_ms {
                    Some(schema)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> SchemaTree {
        SchemaTree::default()
    }

    #[test]
    fn missing_yields_no_live_schema_under_any_freshness() {
        // The bug: `Missing` must not collapse to an empty tree that validity
        // would treat as a real schema. Under both freshness policies it hands
        // back nothing, so the caller classifies LiveSchemaUnavailable.
        assert!(
            SchemaAvailability::Missing
                .live_table_schema(SchemaFreshness::Unbounded)
                .is_none()
        );
        assert!(
            SchemaAvailability::Missing
                .live_table_schema(SchemaFreshness::for_model(0))
                .is_none()
        );
    }

    #[test]
    fn unavailable_yields_no_live_schema_under_any_freshness() {
        // `Unavailable` (store error) must stay distinct from `Missing` in
        // type but classify identically: no live schema, so LiveSchemaUnavailable.
        assert!(
            SchemaAvailability::Unavailable
                .live_table_schema(SchemaFreshness::Unbounded)
                .is_none()
        );
        assert!(
            SchemaAvailability::Unavailable
                .live_table_schema(SchemaFreshness::for_model(0))
                .is_none()
        );
    }

    #[test]
    fn available_within_bound_yields_the_schema() {
        // A cache observed 1h before "now", bound 24h: fresh, so the schema is
        // handed back for classification.
        let one_hour = 60 * 60 * 1000;
        let avail = SchemaAvailability::available(tree(), 0);
        let schema = avail
            .live_table_schema(SchemaFreshness::for_model(one_hour))
            .expect("fresh cache classifies against its schema");
        assert!(schema.databases.is_empty());
    }

    #[test]
    fn available_older_than_the_bound_yields_nothing_for_the_model_path() {
        // A cache observed 25h before "now", bound 24h: too old to vouch for
        // currency on the model path, so no schema — the caller classifies
        // LiveSchemaUnavailable, never Current.
        let now = 25 * 60 * 60 * 1000;
        let avail = SchemaAvailability::available(tree(), 0);
        assert!(
            avail
                .live_table_schema(SchemaFreshness::for_model(now))
                .is_none(),
            "a cache older than the bound must not classify Current"
        );
    }

    #[test]
    fn available_older_than_the_bound_still_classifies_for_a_human() {
        // The human-review path is Unbounded: a 25h-old cache is handed back,
        // because a reviewer is not asked to trust a query built on it.
        let avail = SchemaAvailability::available(tree(), 0);
        assert!(
            avail
                .live_table_schema(SchemaFreshness::Unbounded)
                .is_some(),
            "human path uses a stale-by-age cache to show what it knows"
        );
    }

    #[test]
    fn cache_exactly_at_the_bound_is_still_fresh() {
        // The bound is "older than" — a cache exactly max_age old is fresh.
        let max_age = MODEL_SCHEMA_MAX_AGE_MS;
        let avail = SchemaAvailability::available(tree(), 0);
        assert!(
            avail
                .live_table_schema(SchemaFreshness::for_model(max_age))
                .is_some(),
            "a cache exactly at the bound is fresh, not stale-by-age"
        );
    }
}
