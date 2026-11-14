//! Which settled reservations still need reconciliation, read from the
//! persisted settlement shape.
//!
//! A model request whose usage is unknown (a cut stream, retries exhausted,
//! cancellation after dispatch) cannot change the workspace, so its unknown
//! request, slot, attempt and token observations are not an unresolved
//! effect. Unknown tool, command or child-slot observations are. A settlement
//! persisted as `uncertain` solely because of unknown model usage is read the
//! same way, and nothing is rewritten.

/// The observation dimensions a model request settles.
const USAGE: &str = "('logical_requests','model_request_slots','transport_attempts','input_tokens','output_tokens')";

fn state(event: &str) -> String {
    format!("COALESCE(json_extract({event},'$.event.settlement.effect_state'),'')")
}

fn unknown(event: &str, within: &str) -> String {
    format!(
        "EXISTS(SELECT 1 FROM json_each({event},'$.event.settlement.observations') AS o \
         WHERE json_extract(o.value,'$.value.status')='unknown' \
         AND json_extract(o.value,'$.dimension') {within} {USAGE})"
    )
}

/// True for a settlement with an unknown observation outside the usage dimensions.
pub(super) fn effect_unknown(event: &str) -> String {
    unknown(event, "NOT IN")
}

/// True for a reservation whose settlement is missing, unrecognised, or
/// `uncertain` for any reason other than unknown usage alone.
pub(super) fn unresolved(event: &str) -> String {
    let state = state(event);
    let usage = unknown(event, "IN");
    let effect = unknown(event, "NOT IN");
    format!("({state} != 'settled' AND NOT ({state} = 'uncertain' AND {usage} AND NOT {effect}))")
}
