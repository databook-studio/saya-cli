//! The claim payload contract: the `ClaimPayload` value a fact stores.
//!
//! The type definitions are this module's public surface; the validating
//! constructors live in `constructors` and the read-side derivations a
//! reconciler uses (kind discriminator, referenced columns, tombstoning,
//! column snapshots) live in `derive`.

mod constructors;
mod derive;
mod payload;
mod wire;

pub use payload::{ClaimPayload, ReferencedColumn};

// Imported so the payload module's namespace — which its test modules pull in
// wholesale with `use super::*` — carries the column bound, the contract
// error, and the object-ref type the constructors and tests reference. The
// module's own type definitions don't use them directly; the constructors in
// `constructors` import what they need themselves.
#[allow(unused_imports)]
use crate::contract::{
    claim::MAX_REFERENCED_COLUMNS, error::ContractError, identity::DatabaseObjectRef,
};

#[cfg(test)]
#[path = "../claim_payload_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../claim_payload_property_tests.rs"]
mod property_tests;
