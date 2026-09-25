//! The run record types and the persisted-string gate for the one
//! string-bearing channel the run tables have.
//!
//! The state store is metadata only: a run's goal, plan, and journal live in
//! its run directory, and no parameter of this API accepts free text. The
//! per-endpoint budget and usage maps are keyed by endpoint names — the only
//! caller-supplied strings that are persisted — so their serialized form goes
//! through the same gate every persisted string in this crate gets: keys must
//! be run-scoped names, and a map that structurally resembles a credential is
//! refused, not scrubbed-and-stored.

use std::collections::BTreeMap;

use saya_types::{MAX_BUDGET_ENDPOINTS, MAX_NAME_CHARS, RunFailureCode, RunId};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::contracts::admission;
use crate::runs::status::{RunStatus, RunStepStatus};
use crate::{StoreError, redact};

/// Which capabilities a run was approved for, as flags. The state store
/// records the approval's shape only — destination lists, program allowlists,
/// and endpoint bindings live in the run directory with the rest of the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunCapabilityFlags {
    pub workspace_write: bool,
    pub fetch: bool,
    pub runner: bool,
    pub scratch: bool,
}

/// The ceilings a run was declared with, in the store's integer shape
/// (milliseconds for wall-clock figures). The `Duration`-bearing contract
/// type lives in `saya-types`; the store records plain integers so a read is
/// exact and a `None` ceiling is a NULL column.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunBudgets {
    pub wall_clock_ms: Option<u64>,
    pub tokens_per_endpoint: BTreeMap<String, u64>,
    pub turns: Option<u64>,
    pub tool_calls: Option<u64>,
    pub downloaded_bytes: Option<u64>,
    pub workspace_bytes: Option<u64>,
    pub workspace_files: Option<u64>,
    pub process_count: Option<u64>,
    pub process_time_ms: Option<u64>,
}

/// Accumulated usage. Every unreported figure is `None` — unknown, never
/// zero — so a provider that reports nothing is never presented as having
/// cost nothing. Per-endpoint token totals carry their own `None` for the
/// same reason.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunUsage {
    pub wall_clock_ms: Option<u64>,
    pub tokens_per_endpoint: BTreeMap<String, Option<u64>>,
    pub turns: Option<u64>,
    pub tool_calls: Option<u64>,
}

impl RunUsage {
    /// Every figure unreported: what a fresh run reads back as.
    pub fn unknown() -> Self {
        Self::default()
    }
}

/// What `create_run` accepts: identity, the approved capability flags, and
/// the declared budgets. The run starts `Planned`; approval is a transition,
/// never a creation parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    pub id: RunId,
    pub capabilities: RunCapabilityFlags,
    pub budgets: RunBudgets,
}

/// One run's metadata row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub id: RunId,
    pub status: RunStatus,
    pub failure_code: Option<RunFailureCode>,
    pub capabilities: RunCapabilityFlags,
    pub budgets: RunBudgets,
    pub usage: RunUsage,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

/// One step's metadata row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStepRecord {
    pub run_id: RunId,
    pub step: usize,
    pub status: RunStepStatus,
    pub usage: RunUsage,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

/// A `saya run list` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub id: RunId,
    pub status: RunStatus,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

/// The run-scoped-name shape and bound the endpoint keys must have — the
/// same rules the run contracts apply to endpoint names, re-checked here
/// because the store is the last boundary before the bytes.
fn valid_endpoint_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().count() <= MAX_NAME_CHARS
        && !key.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn validate_endpoint_keys<V>(map: &BTreeMap<String, V>) -> Result<(), StoreError> {
    if map.len() > MAX_BUDGET_ENDPOINTS {
        return Err(StoreError::LimitExceeded);
    }
    if !map.keys().all(|key| valid_endpoint_key(key)) {
        return Err(StoreError::Invalid);
    }
    Ok(())
}

/// Serialise a per-endpoint count map for its JSON column; an empty map is
/// NULL, not `{}`. The serialised form is itself a persisted string, so it
/// goes through the same gate every persisted string gets: a credential- or
/// secret-shaped key is refused at admission, before any INSERT.
pub(crate) fn encode_endpoint_counts<V: Serialize>(
    map: &BTreeMap<String, V>,
) -> Result<Option<String>, StoreError> {
    if map.is_empty() {
        return Ok(None);
    }
    validate_endpoint_keys(map)?;
    let json = serde_json::to_string(map).map_err(|_| StoreError::Invalid)?;
    if redact(&json) != json {
        return Err(StoreError::Invalid);
    }
    admission::check(&json)?;
    Ok(Some(json))
}

/// Decode a stored map. A row this build cannot read — a shape or a bound it
/// does not recognise — fails closed as `Invalid` rather than guessing.
pub(crate) fn decode_endpoint_counts<V: DeserializeOwned>(
    json: Option<String>,
) -> Result<BTreeMap<String, V>, StoreError> {
    let Some(json) = json else {
        return Ok(BTreeMap::new());
    };
    let map: BTreeMap<String, V> = serde_json::from_str(&json).map_err(|_| StoreError::Invalid)?;
    validate_endpoint_keys(&map)?;
    Ok(map)
}

/// Bind helper: a figure beyond the store's integer range is a limit, refused
/// rather than truncated.
pub(crate) fn i64_of(value: Option<u64>) -> Result<Option<i64>, StoreError> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|_| StoreError::LimitExceeded)
}

/// Read helper: a negative figure in a stored row is corruption, refused.
pub(crate) fn u64_of(value: Option<i64>) -> Result<Option<u64>, StoreError> {
    value
        .map(u64::try_from)
        .transpose()
        .map_err(|_| StoreError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::{i64_of, u64_of};
    use std::collections::BTreeMap;

    #[test]
    fn endpoint_maps_round_trip_through_the_codec() {
        let map = BTreeMap::from([
            ("primary".to_owned(), Some(5u64)),
            ("fallback".to_owned(), None),
        ]);
        let json = super::encode_endpoint_counts(&map).unwrap().unwrap();
        let decoded: BTreeMap<String, Option<u64>> =
            super::decode_endpoint_counts(Some(json)).unwrap();
        assert_eq!(decoded, map);
        // An empty map is NULL, not `{}`.
        assert!(
            super::encode_endpoint_counts::<u64>(&BTreeMap::new())
                .unwrap()
                .is_none()
        );
        assert!(
            super::decode_endpoint_counts::<u64>(None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn credential_shaped_keys_are_refused_by_the_codec() {
        for bad in [
            "api_key=sk-live-SENTINEL",
            "password=hunter2",
            "postgres://user:SECRET@host/db",
            "/Users/someone/project",
            "Authorization: Bearer x",
            "bad\u{7}key",
            "bad key",
            "",
        ] {
            let map = BTreeMap::from([(bad.to_owned(), 1u64)]);
            assert_eq!(
                super::encode_endpoint_counts(&map),
                Err(crate::StoreError::Invalid),
                "should refuse {bad:?}"
            );
        }
    }

    #[test]
    fn overwide_maps_exceed_the_store_limit() {
        let mut map = BTreeMap::new();
        for index in 0..9 {
            map.insert(format!("endpoint-{index}"), 1u64);
        }
        assert_eq!(
            super::encode_endpoint_counts(&map),
            Err(crate::StoreError::LimitExceeded)
        );
    }

    #[test]
    fn integer_conversion_refuses_what_it_cannot_represent() {
        assert_eq!(i64_of(Some(5)), Ok(Some(5)));
        assert_eq!(i64_of(None), Ok(None));
        assert_eq!(
            i64_of(Some(u64::MAX)),
            Err(crate::StoreError::LimitExceeded)
        );
        assert_eq!(u64_of(Some(-1)), Err(crate::StoreError::Invalid));
        assert_eq!(u64_of(Some(5)), Ok(Some(5)));
        assert_eq!(u64_of(None), Ok(None));
    }
}
