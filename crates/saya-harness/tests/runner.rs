//! The M5-4 escape battery: the red tests are the deliverable, not the
//! spawn. Every guarantee the runner's design makes is a named test here:
//!
//! 1. A non-allowlisted program is a typed error and nothing is spawned.
//! 2. The argv corpus: `;`, `$()`, backticks, spaces, and newlines each
//!    arrive as exactly one argv element — asserted on what the child
//!    received, never on the absence of a crash.
//! 3. `bash -c`, `sh -c`, a wrapper script, and an absolute path to an
//!    allowlisted program's lookalike are each refused, each as its own
//!    named test.
//! 4. A timeout kills a daemonizing grandchild: the child forks and stays
//!    alive, the grandchild detaches, and the whole group is gone.
//! 5. ~4 GB-equivalent of stdout stays ring-buffered, capped, and the
//!    truncation is reported.
//! 6. A planted database credential is absent from the child's actual
//!    environment, not from the config that was passed.
//! 7. An endpoint credential is present only under all four conditions;
//!    removing any one makes it disappear.
//! 8. Captured output reaches the model and disk only after `redact()`.
//!
//! The refusal battery (tests 1, 3) is pure validation and runs on every
//! platform CI covers; the spawn-dependent tests need the startup probe to
//! prove the sandbox, which — per the M5-3 decision this slice consumes —
//! only happens where the platform's sandbox is provable (macOS today).

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod spawn_battery {
    use std::{
        fs, io,
        path::PathBuf,
        process::Command,
        sync::{Arc, OnceLock},
        time::{Duration, Instant},
    };

    use saya_agent::ToolExecutor;
    use saya_harness::endpoints::endpoint_env_var;
    use saya_harness::runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision};
    use saya_harness::runner::{
        Credential, OutputRing, ProgramOutcome, RUN_PROGRAM_TOOL, RunProgram, RunnerError,
        StaticCredentialSource, refuse::validate_call,
    };
    use saya_types::{CREDENTIAL_ENV_PREFIX, RunnerScope};

    /// The allowlisted runner program: the battery's compiled helper.
    const PROGRAM: &str = "saya-probe";
    /// The tool's configured default timeout in the battery.
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

    /// A directory that lives as long as the test process: the shared
    /// battery provisions are built once and reused by every test.
    fn leak(tag: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("saya-runner-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("battery root must be creatable");
        fs::canonicalize(&path).expect("battery root must canonicalise")
    }

    include!("runner/helper.rs");

    include!("runner/fixtures.rs");

    include!("runner/report.rs");

    // -- test 1: a non-allowlisted program spawns nothing -------------------

    mod refusals {
        include!("runner/refusals.rs");
    }

    // -- test 2: the argv corpus --------------------------------------------

    mod argv {
        include!("runner/argv.rs");
    }

    // -- test 3: the four named shell refusals -------------------------------

    // -- test 4: the timeout kills a daemonizing grandchild -------------------

    mod process_groups {
        include!("runner/process_groups.rs");
    }

    // -- test 5: the ring buffer holds four gigabytes-equivalent --------------

    mod output_capture {
        include!("runner/output_capture.rs");
    }

    // -- test 6: a planted credential is absent from the child's env ----------

    mod credentials {
        include!("runner/credentials.rs");
    }

    // -- test 7: the credential's four conditions ------------------------------

    fn probe_credential() -> Credential {
        use saya_types::SecretRef;
        Credential::new(
            "probe-endpoint",
            // The real generated name (`SAYA_RUN_EP_PROBE_ENDPOINT`, via the
            // same endpoint_env_var the run-config generator uses). The name
            // matches none of the `key=value` markers, so this exercises the
            // env-assignment redaction rule — a stand-in ending in API_KEY
            // would pass for the wrong reason.
            endpoint_env_var("probe-endpoint"),
            SecretRef::Env {
                env: "SAYA_PROBE_SECRET_SOURCE".to_owned(),
            },
        )
        .expect("the declared credential is well-shaped")
    }

    fn credentialed_tool() -> RunProgram {
        let source = StaticCredentialSource::new([(
            "SAYA_PROBE_SECRET_SOURCE".to_owned(),
            "sk-probe-value-123".to_owned(),
        )]);
        standard()
            .tool_with_scope_and_source(
                RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"),
                Arc::new(source),
            )
            .with_credentials(vec![probe_credential()])
    }

    /// Remove "sandboxed": where the probe did not prove the sandbox, no
    /// spawn configuration exists and the tool cannot exist at all — the
    /// credential has no path to a child.
    #[test]
    fn an_unsandboxed_host_has_no_runner_to_inject_through() {
        let workspace = leak("unproven-ws");
        let programs = leak("unproven-programs");
        let policy = RunSandbox::new([workspace.clone()], Vec::<(String, u16)>::new())
            .expect("the policy constructs while the root exists");
        fs::remove_dir_all(&workspace).expect("the root vanishes before prepare");
        let provision = policy.prepare(&programs).expect("preparation runs");
        assert!(provision.spawn().is_none(), "no proven sandbox, no spawn");
        assert!(
            !provision.report().failed_required().is_empty(),
            "the report must name what failed"
        );
    }

    // -- test 8: redact() gates the model copy and the disk copy ---------------

    // -- the rest of the shape: narrowing, cwd, exit codes, cancellation -------

    mod execution_shape {
        include!("runner/execution_shape.rs");
    }
}
