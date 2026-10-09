//! The approval-surface split: what each surface states about itself, and
//! what the advertisement gate reads.
//!
//! `can_prompt` means "this surface may read stdin" — the TUI always answers
//! false, and must, under the alternate screen. `can_obtain_approval` means
//! "this surface can obtain a per-call approval at all" — the TUI answers
//! true through its modal. [`SessionUniverse::definitions`] consumes the
//! second, never the first: a surface that cannot read stdin but can still
//! ask advertises exactly what the line REPL advertises. These tests pin the
//! split from both ends — the surfaces' stated values and the gate's reading
//! of them — so the two flags cannot be merged back into one.

use std::{collections::BTreeSet, fs};

use saya_agent::{AgentMode, ApprovalPolicy};

use crate::interactive::tui::agent::approval_capabilities;

#[path = "approval_surface_tests/test_support.rs"]
mod test_support;
use test_support::{
    WRITE_SHAPED, advertised_set, assert_host_tool_for_platform, compose, temp_dir, worktree,
};

// ---------------------------------------------------------------------------
// the split, pinned from both ends
// ---------------------------------------------------------------------------

/// The TUI states the truth about itself: it must never read stdin under
/// the alternate screen, and it can always obtain a per-call approval
/// through its modal. `start` passes exactly this pair — if either half
/// drifts, the parity test below fails.
#[test]
fn tui_states_it_cannot_read_stdin_but_can_obtain_approvals() {
    assert_eq!(
        approval_capabilities(),
        (false, true),
        "the TUI never reads stdin and always has its modal"
    );
}

/// The regression the user hit: a TUI-shaped session — bound workspace,
/// `ask`, Build — advertises `workspace_write` and the rest of the
/// write-shaped set. Under the single-flag reading the TUI's honest
/// "cannot read stdin" classified it as having no approval surface at all,
/// so every write-shaped tool stayed hidden.
#[test]
fn tui_shaped_session_under_ask_advertises_the_write_shaped_tools() {
    let project = worktree("tui-regression");
    let state = temp_dir("tui-regression-state");
    let universe = compose(&project, &state);
    let (_, tui_can_obtain_approval) = approval_capabilities();
    let names = advertised_set(
        &universe,
        AgentMode::Build,
        ApprovalPolicy::Ask,
        tui_can_obtain_approval,
    );
    for tool in WRITE_SHAPED
        .into_iter()
        .filter(|tool| *tool != "run_command")
    {
        assert!(
            names.contains(tool),
            "a TUI-shaped ask session advertises {tool}: {names:?}"
        );
    }
    assert_host_tool_for_platform(&names);
    // Advertised is not allowed: the definition still declares its approval
    // shape honestly, and the engine — untouched by this slice — decides
    // every call, which the modal answers per call.
    let workspace_write = universe
        .definitions(
            AgentMode::Build,
            ApprovalPolicy::Ask,
            tui_can_obtain_approval,
            true,
            false,
            false,
        )
        .into_iter()
        .find(|definition| definition.name == "workspace_write")
        .expect("workspace_write is advertised");
    assert!(
        workspace_write.effect.requires_approval,
        "workspace_write stays ask-gated: the modal answers every call"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The owner's ruling, expressed as a test: the TUI's advertised set under
/// `ask` equals the line REPL's advertised set for the same universe and
/// policy — set equality over tool names. The TUI's half is read off the
/// call site's own statement (`approval_capabilities`); the REPL's half is
/// the live-terminal fact the headless loop passes for both flags.
#[test]
fn tui_and_repl_advertise_identical_sets_under_ask() {
    let project = worktree("tui-repl-parity");
    let state = temp_dir("tui-repl-parity-state");
    let universe = compose(&project, &state);
    let (_, tui_can_obtain_approval) = approval_capabilities();
    let repl_can_obtain_approval = true;
    let tui: BTreeSet<String> = advertised_set(
        &universe,
        AgentMode::Build,
        ApprovalPolicy::Ask,
        tui_can_obtain_approval,
    );
    let repl: BTreeSet<String> = advertised_set(
        &universe,
        AgentMode::Build,
        ApprovalPolicy::Ask,
        repl_can_obtain_approval,
    );
    assert_eq!(
        tui, repl,
        "the TUI is a prompting surface: under ask it advertises exactly what the REPL does"
    );
    assert!(
        tui.contains("workspace_write"),
        "both surfaces advertise the write the user asked for: {tui:?}"
    );
    assert!(
        tui.contains("workspace_edit"),
        "both surfaces advertise the guarded edit: {tui:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// `read-only` and `never` still advertise no write-shaped tool on any
/// surface — even one that can obtain approvals. The policy, not the
/// surface, decides there: neither policy can ever allow a write-shaped
/// call, so the tools stay hidden.
#[test]
fn read_only_and_never_hide_write_shaped_tools_on_every_surface() {
    let project = worktree("ro-never-every-surface");
    let state = temp_dir("ro-never-every-surface-state");
    let universe = compose(&project, &state);
    for mode in [ApprovalPolicy::ReadOnly, ApprovalPolicy::Never] {
        for can_obtain_approval in [false, true] {
            let names = advertised_set(&universe, AgentMode::Build, mode, can_obtain_approval);
            for tool in WRITE_SHAPED {
                assert!(
                    !names.contains(tool),
                    "{mode:?} (can_obtain_approval={can_obtain_approval}) must not advertise {tool}: {names:?}"
                );
            }
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// `bypass` advertises the full write-shaped set on both surfaces: bypass
/// **is** the consent, so no approval surface is needed.
#[test]
fn bypass_advertises_the_full_write_shaped_set_on_both_surfaces() {
    let project = worktree("bypass-both-surfaces");
    let state = temp_dir("bypass-both-surfaces-state");
    let universe = compose(&project, &state);
    for can_obtain_approval in [false, true] {
        let names = advertised_set(
            &universe,
            AgentMode::Build,
            ApprovalPolicy::Bypass,
            can_obtain_approval,
        );
        for tool in WRITE_SHAPED
            .into_iter()
            .filter(|tool| *tool != "run_command")
        {
            assert!(
                names.contains(tool),
                "bypass advertises {tool} with or without an approval surface \
                 (can_obtain_approval={can_obtain_approval}): {names:?}"
            );
        }
        assert_host_tool_for_platform(&names);
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Plan mode still hides write-shaped tools on both surfaces: the task
/// posture judges what the task may touch, whatever the consent posture.
#[test]
fn plan_hides_write_shaped_tools_on_both_surfaces() {
    let project = worktree("plan-both-surfaces");
    let state = temp_dir("plan-both-surfaces-state");
    let universe = compose(&project, &state);
    for can_obtain_approval in [false, true] {
        let names = advertised_set(
            &universe,
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            can_obtain_approval,
        );
        for tool in WRITE_SHAPED {
            assert!(
                !names.contains(tool),
                "Plan hides {tool} even with an approval surface \
                 (can_obtain_approval={can_obtain_approval}): {names:?}"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The anti-pattern, still pinned: a surface that can obtain no approval at
/// all — neither stdin nor a modal — must not advertise write-shaped tools
/// under `ask`. That rule survives the split unchanged; only which surfaces
/// qualify changed. `tasks_set` is the one exception that survives with it:
/// session metadata the engine admits even there.
#[test]
fn a_surface_with_no_approval_surface_advertises_nothing_write_shaped() {
    let project = worktree("no-approval-surface");
    let state = temp_dir("no-approval-surface-state");
    let universe = compose(&project, &state);
    let names = advertised_set(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in WRITE_SHAPED {
        assert!(
            !names.contains(tool),
            "a surface with no approval surface must not advertise {tool}: {names:?}"
        );
    }
    assert!(
        names.contains("tasks_set"),
        "tasks_set is the exception the engine admits: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
