// The interactive session's `shell`: the shared tool
// (`agent::tools::shell_policy`) composed over the session's own workspace
// root, so a session's command door is the same one the coding root and a run
// advertise.
//
// Two facts make a session's shell differ from theirs, and both are here
// rather than in the shared module:
//
// - **Its mode and its operator are the turn's.** A session's mode can change
//   mid-session (`/approvals`), and its operator — the decider that asks the
//   user — is built per turn. So the shell itself is composed per turn
//   (`SessionUniverse::shell`) rather than once per process, and the sandbox
//   it runs in is the only part proven once (`ShellSandboxes`, which caches
//   each shape the probe can produce). The background registry is the
//   session's, though: a detached run started in one turn is read from the
//   next, and ends with the session rather than with the turn's shell.
// - **Its advertisement is by mode.** `ask`, `bypass` and `read-only` all keep
//   a lane — `read-only`'s is the sandbox, where nothing in the checkout is
//   writable — and `never` keeps none. Plan hides it with the rest of the
//   write-shaped surface, whose posture denies an external side effect.

use std::path::PathBuf;
use std::sync::OnceLock;

use saya_agent::ApprovalDecider;
use saya_harness::runner::sandbox::RunnerSpawn;

/// The session's proven shell sandboxes, one per shape: the writable one every
/// mode but `read-only` serves a sandboxed call from, and the write-denied one
/// `read-only` alone uses. Each is proven at most once per session — the probe
/// spawns canaries, so it must never run per turn — and lazily, because a
/// session's mode can change mid-session, so which shapes are needed is not
/// known when the session is composed. A shape that does not prove is `None`
/// forever: the verdict is a fact about this host, not a transient.
struct ShellSandboxes {
    /// The canonical workspace root the probe runs against — the same root
    /// the composed shell confines its children to.
    root: PathBuf,
    writable: OnceLock<Option<RunnerSpawn>>,
    read_only: OnceLock<Option<RunnerSpawn>>,
}

impl ShellSandboxes {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            writable: OnceLock::new(),
            read_only: OnceLock::new(),
        }
    }

    /// The sandbox this mode's shell runs in, proven once per session. The
    /// shape is the mode's: `read-only` write-denies the whole root, so even a
    /// sandboxed command can change nothing; every other mode denies nothing.
    fn prove(&self, mode: ApprovalPolicy) -> Option<&RunnerSpawn> {
        let cell = if mode == ApprovalPolicy::ReadOnly {
            &self.read_only
        } else {
            &self.writable
        };
        cell.get_or_init(|| shell_policy::prove(&self.root, mode))
            .as_ref()
    }
}

impl SessionUniverse {
    /// This turn's `shell`, or `None` where this session has no shell at all.
    ///
    /// Composed per turn because the mode, the operator (the turn's decider,
    /// when this surface can obtain an approval at all) and the cancellation
    /// are all the turn's facts; the sandbox underneath it was proven once per
    /// session ([`ShellSandboxes`]). The lane decision is the shared
    /// [`ShellPolicy`](shell_policy::ShellPolicy)'s: a `sandbox: true` call
    /// runs in the proven sandbox without asking, and a host call reaches the
    /// mode — run outright under `bypass`, put to the operator under `ask`,
    /// served from the sandbox under `read-only`, and refused with a reason
    /// where the mode keeps no lane for it.
    ///
    /// `None` mirrors the shared [`compose`](shell_policy::compose): no
    /// workspace root, mode `never`, or `read-only` on a host where the
    /// sandbox does not prove — a read-only session has no shell that could
    /// reach outside one.
    pub(crate) fn shell(
        &self,
        mode: ApprovalPolicy,
        operator: Option<Arc<dyn ApprovalDecider>>,
        cancellation: CancellationToken,
    ) -> Option<Arc<ShellTool>> {
        if !shell_policy::advertised(mode) {
            return None;
        }
        let sandboxes = self.shell_sandboxes.as_ref()?;
        // The session's approval surface is an `ApprovalDecider`; the shared
        // prompt takes an `Operator`, so the surface is bridged here. Its
        // prompt is its own — the subject names no coding job.
        let prompt = operator.map(|decider| {
            shell_policy::prompt(
                Arc::new(shell_policy::DeciderShellOperator::new(decider)),
                crate::agent::tools::consent::Caller::Session,
            )
        });
        let placement = shell_policy::ShellPlacement::root(&sandboxes.root);
        match sandboxes.prove(mode) {
            Some(spawn) => Some(Arc::new(shell_policy::compose_sandboxed(
                placement,
                mode,
                spawn.clone(),
                prompt,
                self.shell_host.clone(),
                cancellation,
                // The shell is composed per turn, but its background runs
                // belong to the session: every turn reaches the same registry,
                // and the session's token — not this turn's — ends them.
                shell_policy::Background::Shared {
                    runs: Arc::clone(&self.shell_background),
                    cancellation: self.shell_background_cancellation.clone(),
                },
                self.shell_rules.clone(),
            ))),
            None if mode == ApprovalPolicy::ReadOnly => None,
            None => shell_policy::compose_host_only(
                &sandboxes.root,
                mode,
                prompt,
                self.shell_host.clone(),
                cancellation,
                shell_policy::Background::Shared {
                    runs: Arc::clone(&self.shell_background),
                    cancellation: self.shell_background_cancellation.clone(),
                },
                None,
                self.shell_rules.clone(),
            )
            .map(Arc::new),
        }
    }

    /// Whether this session advertises `shell` under `mode`: a workspace root
    /// binds, the mode keeps a lane, and that lane can actually serve a call.
    /// The asking modes (`ask`, `accept-edits`) serve one through the approval
    /// surface (`can_obtain_approval`) or, headless, from the sandbox —
    /// neither pre-answers a shell; `bypass` runs it outright; `read-only`'s
    /// only lane is the sandbox, so it advertises exactly where one proves;
    /// `never` keeps no lane. Plan never reaches here — the caller gates on
    /// Build.
    pub(crate) fn shell_available(&self, mode: ApprovalPolicy, can_obtain_approval: bool) -> bool {
        if !shell_policy::advertised(mode) {
            return false;
        }
        let Some(sandboxes) = self.shell_sandboxes.as_ref() else {
            return false;
        };
        match mode {
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => {
                can_obtain_approval || sandboxes.prove(mode).is_some()
            }
            ApprovalPolicy::Bypass => true,
            ApprovalPolicy::ReadOnly => sandboxes.prove(mode).is_some(),
            ApprovalPolicy::Never => false,
        }
    }
}
