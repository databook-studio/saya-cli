//! The platform shell's identity and command-string spelling: which program
//! the workspace shell execs, the `PATH` a sandboxed child gets, and the one
//! helper that hands the command string to that program.

/// The one program this tool execs. A path, not a name: the shell is the
/// allowlisted program, and the sandbox profile is what confines what it can
/// then reach. Windows has no POSIX shell in its base install, so the
/// workspace shell there is the system `cmd.exe`; the host lane is
/// unsandboxed on both platforms, and Windows never claims a confinement.
#[cfg(not(windows))]
pub const SHELL_PROGRAM: &str = "/bin/sh";
#[cfg(windows)]
pub const SHELL_PROGRAM: &str = "cmd.exe";

/// The child's whole `PATH`. `/bin` holds `sh` and the shell's own utilities;
/// `/usr/bin` holds the programs the tests and a real command need — no single
/// directory holds both, which is why the spawn is prepared with all of them.
/// Windows has no sandboxed shell path: the host lane inherits the caller's
/// own `PATH`.
#[cfg(not(windows))]
pub const SHELL_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
#[cfg(windows)]
pub const SHELL_PATH: &str = "";

/// The argv prefix that tells the shell to run one command string: Unix `-c`,
/// with the command as the next argv element; Windows `cmd.exe`'s `/d`
/// (ignore AutoRun), `/s` (preserve the command's own quotes) and `/c` (run
/// and exit), with the command appended by [`ShellCommandLine`].
#[cfg(not(windows))]
pub(crate) fn shell_argv() -> Vec<String> {
    vec!["-c".to_owned()]
}
#[cfg(windows)]
pub(crate) fn shell_argv() -> Vec<String> {
    vec!["/d".to_owned(), "/s".to_owned(), "/c".to_owned()]
}

/// Appends the shell's command string to a command line.
///
/// Unix takes it as one ordinary argv element after `-c`. Windows cannot:
/// `cmd.exe /c` takes the rest of its command line as the command, and
/// Rust's default MSVC argument quoting would escape the command's own double
/// quotes (`"a + b"` becomes `\"a + b\"`), which `cmd.exe` does not
/// understand — `/s` then strips only the outer pair and runs the backslashes
/// literally. The command is therefore appended as **one raw argument**
/// wrapped in the single outer pair `/s` strips, so the shell sees exactly
/// the caller's bytes.
///
/// Both command types go through this one helper, so no spawn site can
/// regress to ordinary argument quoting.
pub(crate) trait ShellCommandLine {
    fn shell_command(&mut self, command: &str) -> &mut Self;
}

impl ShellCommandLine for std::process::Command {
    fn shell_command(&mut self, command: &str) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            self.raw_arg(format!("\"{command}\""))
        }
        #[cfg(not(windows))]
        {
            self.arg(command)
        }
    }
}

impl ShellCommandLine for tokio::process::Command {
    fn shell_command(&mut self, command: &str) -> &mut Self {
        self.as_std_mut().shell_command(command);
        self
    }
}

/// The exact shell program path the child is spawned with. On Windows this is
/// the system `cmd.exe`, located through the system directory — never `PATH`,
/// `ComSpec`, or the current directory, all of which a caller can influence.
#[cfg(not(windows))]
pub(crate) fn shell_program_path() -> std::path::PathBuf {
    std::path::PathBuf::from(SHELL_PROGRAM)
}
#[cfg(windows)]
pub(crate) fn shell_program_path() -> std::path::PathBuf {
    crate::proc::system_shell()
}
