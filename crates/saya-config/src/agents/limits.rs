/// The most helper children one job runs at once. A child is a whole model
/// loop over the job's own checkout, so the bound is small on purpose.
pub(super) const MAX_CHILDREN: usize = 3;
/// The most model requests in flight across the job and its helpers. At least
/// two, so a coordinator and one helper can both be in flight.
pub(super) const MAX_CONCURRENT_MODEL_REQUESTS: usize = 4;
/// The most helper types one `[agents]` table may declare; the two built-ins are
/// not counted. The registry is advertised whole in the `helpers` schema, so the
/// table is bounded like every other config list.
pub(super) const MAX_HELPER_AGENTS: usize = 16;
/// The longest helper type name: it is advertised in the tool schema and echoed
/// back in a refusal.
pub(super) const MAX_HELPER_AGENT_NAME_CHARS: usize = 32;

/// The two keys in `[agents]` that are settings, not helper type names. They are
/// reserved: a key in that table means one thing, and a type named
/// `max_children` would make it mean two. Enforced twice — once on the text, so
/// the refusal names the reserved key, and once on the registry, so a
/// programmatically built table cannot slip one past.
pub(crate) const RESERVED_AGENT_SETTINGS: [&str; 2] =
    ["max_children", "max_concurrent_model_requests"];
