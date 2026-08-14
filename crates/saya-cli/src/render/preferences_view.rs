//! Render-owned preference view DTOs.
//!
//! Presentation types only, mirroring `contract_view.rs`: the store returns
//! typed `PreferenceValue`s paired with a `PreferenceScope` that *contains* the
//! opaque `ProfileIdentity`. This DTO carries the scope as a **profile name**
//! (or `"global"`), never the identity. There is no field for the identity here,
//! and there must not be one — a structural guarantee, not a convention to
//! remember (see `preferences_view_serialized_keys_exclude_opaque_identity`).
//!
//! `value` is the short rendered form of the preference value (the timezone
//! string, the grain, the style, the profile name). `kind` is the stable
//! discriminator (`timezone`, `date_grain`, `output_style`, `default_profile`).

use serde::{Deserialize, Serialize};

/// One preference, in renderable form. `scope` is the profile *name* for a
/// profile-scoped preference, or `"global"` for a global one — never the opaque
/// `ProfileIdentity`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreferenceView {
    pub kind: String,
    pub value: String,
    pub scope: String,
}
