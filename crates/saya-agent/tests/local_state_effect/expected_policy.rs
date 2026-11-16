/// The independent expected policy for the public boundary matrix. A refused
/// approval has precedence even when the same declaration is structurally
/// restricted; a grant still cannot override any structural restriction.
fn expected_policy(
    effect: ToolEffect,
    limits: AgentLimits,
    approved: bool,
) -> (&'static str, Option<&'static str>) {
    if effect.requires_approval && !approved {
        ("denied", Some("approval was not granted"))
    } else if effect.external_side_effect
        && !effect.requires_approval
        && !limits.permit_external_effects
    {
        ("denied", Some("external side effect requires approval"))
    } else if effect.local_state == LocalStateEffect::WriteCandidate
        && !limits.permit_candidate_writes
    {
        ("denied", Some("candidate writes are not permitted"))
    } else if effect.local_state == LocalStateEffect::WriteWorkspace
        && !limits.permit_workspace_writes
    {
        ("denied", Some("workspace writes are not permitted"))
    } else {
        ("completed", None)
    }
}
