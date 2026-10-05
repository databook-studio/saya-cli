//! The `/context` view: what the next request would load from the two
//! instruction sources — names, sizes, digests, flags, and skip/omit
//! reasons, never the bodies. One shared renderer for both surfaces.

use crate::agent::context_sources::{
    InstructionProjection, MAX_INSTRUCTION_REQUEST_BYTES, project_instructions,
};
use crate::config::runtime::RuntimeConfig;
use saya_config::{PERSONAL_INSTRUCTIONS_FILE, PROJECT_INSTRUCTIONS_FILE, ResolvedInstructions};
use saya_harness::instructions::{InstructionOrigin, SkipReason};
use std::path::Path;

#[cfg(test)]
#[path = "session_context_tests.rs"]
mod session_context_tests;

/// Builds the projection exactly as the live request would — with the
/// full instruction budget, so the view shows what can load, not what a
/// crowded turn would trim — and renders the shared text.
pub(crate) fn context_view(runtime: &RuntimeConfig, workspace_root: Option<&Path>) -> String {
    let projection = project_instructions(
        &runtime.resolved.instructions,
        workspace_root,
        &crate::config::sources::user_config_dir(),
        MAX_INSTRUCTION_REQUEST_BYTES,
    );
    render(&runtime.resolved.instructions, &projection)
}

/// The pure renderer: settings, admitted receipts, skips, omissions, and
/// the one bound that matters — guidance, never permissions, never text.
fn render(settings: &ResolvedInstructions, projection: &InstructionProjection) -> String {
    let mut out =
        String::from("Instruction sources for the next request\n\nSettings: project instructions ");
    out.push_str(if settings.project_enabled {
        "on"
    } else {
        "off"
    });
    out.push_str(", personal instructions ");
    out.push_str(if settings.personal_enabled {
        "on"
    } else {
        "off"
    });
    out.push_str(&format!(
        "; limits: project {} B, personal {} B, total {} B.",
        settings.project_max_bytes, settings.personal_max_bytes, settings.total_max_bytes,
    ));
    if projection.sources.is_empty()
        && projection.skipped.is_empty()
        && projection.omitted.is_empty()
    {
        out.push_str("\n\n");
        out.push_str(&nothing_found_sentence(settings));
    }
    if !projection.sources.is_empty() {
        out.push_str("\n\nLoaded:");
        for source in &projection.sources {
            out.push_str(&format!(
                "\n  {} {} — {} B file, {} B included, sha256 {}, truncated: {}, redactions: {}",
                origin_label(source.origin),
                source.file_name,
                source.file_size,
                source.included_bytes,
                short_digest(&source.sha256),
                if source.truncated { "yes" } else { "no" },
                source.redactions,
            ));
        }
    }
    if !projection.skipped.is_empty() {
        out.push_str("\n\nSkipped:");
        for source in &projection.skipped {
            out.push_str(&format!(
                "\n  {} {} — {}",
                origin_label(source.origin),
                file_of(source.origin),
                reason_text(source.reason),
            ));
        }
    }
    if !projection.omitted.is_empty() {
        out.push_str("\n\nOmitted:");
        for source in &projection.omitted {
            out.push_str(&format!(
                "\n  {} {} — loaded, but the next request had no byte budget for it",
                origin_label(source.origin),
                source.file_name,
            ));
        }
    }
    out.push_str(
        "\n\nThese files carry project guidance, never permissions, and their text is \
         never shown here.",
    );
    out
}

/// The nothing-found sentence, honest about which sources were looked at.
fn nothing_found_sentence(settings: &ResolvedInstructions) -> String {
    match (settings.project_enabled, settings.personal_enabled) {
        (true, true) => format!(
            "No instruction sources found: looked for {PROJECT_INSTRUCTIONS_FILE} in the \
             workspace root and {PERSONAL_INSTRUCTIONS_FILE} in your config directory."
        ),
        (true, false) => format!(
            "No instruction sources found: looked for {PROJECT_INSTRUCTIONS_FILE} in the \
             workspace root; personal instructions are disabled in settings."
        ),
        (false, true) => format!(
            "No instruction sources found: looked for {PERSONAL_INSTRUCTIONS_FILE} in your \
             config directory; project instructions are disabled in settings."
        ),
        (false, false) => "No instruction sources found: project and personal instruction \
                           sources are both disabled in settings."
            .to_string(),
    }
}

fn origin_label(origin: InstructionOrigin) -> &'static str {
    match origin {
        InstructionOrigin::Personal => "personal",
        InstructionOrigin::Project => "project",
    }
}

/// A skipped source carries only its origin, but each origin names exactly
/// one file — the loader's own constants.
fn file_of(origin: InstructionOrigin) -> &'static str {
    match origin {
        InstructionOrigin::Personal => PERSONAL_INSTRUCTIONS_FILE,
        InstructionOrigin::Project => PROJECT_INSTRUCTIONS_FILE,
    }
}

fn reason_text(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::SymlinkRefused => "symlink refused (symlinks are never followed)",
        SkipReason::NotRegularFile => "not a regular file",
        SkipReason::TooLarge => "too large for the configured limit",
        SkipReason::NotUtf8 => "not valid UTF-8",
        SkipReason::Unreadable => "unreadable",
    }
}

/// The digest is always 64 hex chars; the first 12 identify the file.
fn short_digest(sha256: &str) -> &str {
    sha256.get(..12).unwrap_or(sha256)
}
