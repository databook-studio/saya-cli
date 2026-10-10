/// The observed lifecycle of one command process owned by the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandProcessState {
    /// No operating-system process has been spawned yet.
    Pending,
    /// `Command::spawn` returned this child PID; this is an ownership snapshot,
    /// not an independent kernel liveness probe.
    Spawned { pid: u32 },
    /// The shared wait reaped this child and completed its process-group sweep.
    Reaped { pid: u32 },
    /// The command failed or was cancelled before an operating-system spawn.
    NotSpawned,
    /// The process was spawned, but the shared wait could not confirm cleanup.
    Unknown { pid: u32 },
}
