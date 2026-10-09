//! The M5-7 credential-isolation end-to-end battery: the planted-secret
//! tests, not the spawn. The claim the design makes is narrow and is tested
//! here rather than asserted: **a credential reaches a child only when it
//! was declared, the child is sandboxed, the generated config holds
//! references only, and captured output is redacted.** Remove any one
//! condition and the credential must not appear.
//!
//! What this battery adds over the M5-4 escape battery (`tests/runner.rs`),
//! which owns the four-condition rule at the spawn level and is read, not
//! duplicated:
//!
//! 1. **The full sweep.** A resolved credential must appear in exactly one
//!    place — the child's environment, proven by a digest the child computes
//!    of its own env value (the raw value is never printed in this test, so
//!    the sweep's only expected hit is the environment itself). The whole
//!    run directory is scanned recursively — generated configs, the journal,
//!    the runner's disk records — plus the store database with its WAL
//!    sidecars, plus the tool result the model actually received. Any other
//!    hit fails.
//! 2. **The exfiltration shape.** A child that echoes its own environment —
//!    `SAYA_RUN_EP_ORCHESTRATOR=<value>`, under the very name the generated
//!    config binds — is the credential leaving through captured output. The
//!    model lane (the `tool` message the loop hands the provider) and the
//!    disk lane (the record the runner persists) are asserted separately;
//!    they are different code paths. This test found a real hole: pattern
//!    redaction alone let the exact injected value through, and the capture
//!    boundary now scrubs the resolved values themselves (see the M5-7
//!    report).
//! 3. **The four conditions end to end** — through the episode loop, the
//!    engine sink, the journal, and the store, not only at the spawn call.
//!    "Declared" and "references-only" are re-proven here; "sandboxed" is
//!    structural and has no end-to-end variant to cover (no proven
//!    `RunnerSpawn`, no tool, no credential list to attach — the escape
//!    battery's construction refusal is the whole of that proof); "redact"
//!    is test 2.
//! 4. **Role separation.** A reviewer credential is absent from an
//!    orchestrator child's environment and the other way round, within one
//!    run driven step by step, with the generated config binding each role
//!    to its own run-scoped env variable.
//! 5. **The failure path.** A run that fails mid-flight (the provider fails
//!    after the child ran; the bounded retry spends itself; the run pauses)
//!    leaves a journal and a store whose bytes carry no credential material.
//!
//! Unproven here, and said so rather than tested vacuously: the nested-saya
//! leg — the composition root does not yet attach the runner tool to a run's
//! toolset, so no nested `saya` child holds a runner credential and there is
//! no path to exercise (see the M5-7 report). The startup probe proves the
//! sandbox on macOS only; elsewhere the runner is absent and these tests
//! compile to nothing rather than pass vacuously.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod battery {
    use std::{
        collections::VecDeque,
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, Mutex, OnceLock},
        time::Duration,
    };

    use async_trait::async_trait;
    use saya_agent::{
        ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
        ProviderError, ToolCall, ToolDefinition, ToolExecutor,
    };
    use saya_harness::endpoints::{
        CorpusProfile, EndpointSpec, ORCHESTRATOR_ROLE, endpoint_env_var, write_run_configs,
    };
    use saya_harness::engine::{
        EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeError, EpisodeRequest,
        EpisodeRun, ManifestBounds, RunState, SinkBudgets, StepToolset, UsageTotals,
    };
    use saya_harness::journal::{EVENTS_FILE, Journal};
    use saya_harness::run_dir::RunDir;
    use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
    use saya_harness::runner::{Credential, RUN_PROGRAM_TOOL, RunProgram, StaticCredentialSource};
    use saya_harness::workspace::Workspace;
    use saya_store::{
        NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStepStatus, RunStore,
        SqliteStateStore,
    };
    use saya_types::{
        Capabilities, EndpointBindings, PauseReason, RunEvent, RunId, RunPlan, RunnerScope,
        SecretRef, StepSpec,
    };

    mod cell {
        include!("credential_isolation/battery/cell.rs");
    }
    mod driver {
        include!("credential_isolation/battery/driver.rs");
    }
    mod environment {
        include!("credential_isolation/battery/environment.rs");
    }
    mod helper {
        include!("credential_isolation/battery/helper.rs");
    }
    mod redaction {
        include!("credential_isolation/battery/redaction.rs");
    }
    mod role_separation {
        include!("credential_isolation/battery/role_separation.rs");
    }
    mod sweep {
        include!("credential_isolation/battery/sweep.rs");
    }

    use self::cell::*;
    use self::driver::*;
    use self::helper::*;
    use self::sweep::*;
}
