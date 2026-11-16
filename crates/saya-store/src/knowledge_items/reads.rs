//! Read-by-profile and read-by-object for the knowledge-items repository.
//!
//! Object identity is inlined on the row, so profile/object reads need no join.
//! Page queries use deterministic keyset cursors; the compatibility helpers
//! walk those bounded pages when an older caller still asks for a `Vec`.
//! Profile scoping is absolute — the profile id is bound in every query, so a
//! read for one profile can never return another's rows.

mod by_id;
mod object;
mod objects;
mod profile;
mod row;

pub(crate) use by_id::read_by_id;
pub(crate) use object::{read_for_object, read_for_object_page};
pub(crate) use objects::{read_objects_for_profile, read_objects_for_profile_page};
pub(crate) use profile::{read_for_profile, read_for_profile_page};
