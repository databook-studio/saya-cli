//! The child coding-command executor answers deterministic pre-effect
//! refusals as tool data; only errors that leave a real command's outcome
//! unknown stay errors. A refusal that happened before any process existed
//! can never make an effect uncertain, so settling it Unknown would fail a
//! job for a refusal the model can simply correct.

use super::command_refusal_data;
use crate::commands::coding::CodingError;
use serde_json::json;

#[test]
fn start_refusals_are_data() {
    let busy = command_refusal_data("coding_command_start", &CodingError::Busy)
        .expect("the two-session cap refuses before any spawn, so it is data");
    assert_eq!(busy["outcome"], json!("refused"));
    assert_eq!(busy["reason"], json!("coding work is already running"));

    let budget = CodingError::owner(std::io::Error::other("command budget admission failed"));
    let data = command_refusal_data("coding_command_start", &budget)
        .expect("a refused slot admission spawns no process, so it is data");
    assert_eq!(data["outcome"], json!("refused"));
    assert_eq!(data["reason"], json!("native coding operation failed"));
}

#[test]
fn refusal_reason_never_carries_the_owner_source_chain() {
    // An owner error can wrap any filesystem, checkout, confinement or I/O
    // failure; its source text is an internal path or secret-shaped string
    // the model must never receive as tool data.
    let cause = std::io::Error::other(
        "open /Users/alice/.saya/state/jobs/7/checkout failed: token=sk-live-0123456789",
    );
    let wrapped = CodingError::owner(std::io::Error::other(cause));
    for name in ["coding_command_start", "coding_command_read"] {
        let data = command_refusal_data(name, &wrapped).expect("pre-effect refusal is data");
        let reason = data["reason"].as_str().expect("reason text");
        assert_eq!(reason, "native coding operation failed");
        for leaked in ["/Users/alice", "sk-live", "token=", "checkout failed"] {
            assert!(!reason.contains(leaked), "{leaked} leaked: {reason}");
        }
    }
}

#[test]
fn read_refusals_are_data() {
    let data = command_refusal_data("coding_command_read", &CodingError::NotOwned)
        .expect("an unknown command id is refused before anything is read, so it is data");
    assert_eq!(data["outcome"], json!("refused"));
    assert_eq!(
        data["reason"],
        json!("coding job is not owned by this application or has no active runtime")
    );
}

#[test]
fn stop_refuses_only_pre_effect_ids_as_data() {
    let data = command_refusal_data("coding_command_stop", &CodingError::NotOwned)
        .expect("stopping an unknown id performed no effect, so it is data");
    assert_eq!(data["outcome"], json!("refused"));

    let joined = CodingError::owner(std::io::Error::other("join failed"));
    assert!(
        command_refusal_data("coding_command_stop", &joined).is_none(),
        "a stop that joined a real command may leave its outcome unknown"
    );
}

#[test]
fn other_tools_keep_their_errors() {
    assert!(
        command_refusal_data("coding_check", &CodingError::Busy).is_none(),
        "only the supervised-command surface classifies refusals"
    );
    assert!(command_refusal_data("coding_notes", &CodingError::NotOwned).is_none());
}

#[test]
fn an_unusable_extension_program_is_refused_as_data_before_any_spawn() {
    // The program path is checked before a server process exists, so the
    // refusal can never make an effect uncertain and the model can correct it.
    for name in ["coding_mcp", "coding_lsp"] {
        let data = command_refusal_data(name, &CodingError::ExtensionProgramRefused)
            .expect("a pre-spawn program refusal is data");
        assert_eq!(data["outcome"], json!("refused"));
        let reason = data["reason"].as_str().expect("reason text");
        assert!(
            reason.contains("symlink"),
            "the reason names the cause: {reason}"
        );
        assert!(
            command_refusal_data(name, &CodingError::Busy).is_none(),
            "only the program refusal is known to precede the spawn"
        );
    }
}
