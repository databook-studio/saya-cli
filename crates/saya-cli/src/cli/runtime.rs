use clap::Subcommand;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum RuntimeCheckAction {
    #[value(name = "test-ci")]
    TestCi,
    Build,
}

#[derive(Debug, Clone, Subcommand)]
pub enum RuntimeCommand {
    /// Install Saya's pinned Node.js and npm runtime.
    InstallNode {
        /// Confirm the fixed managed-runtime installation.
        #[arg(long)]
        yes: bool,
    },
    /// Acquire lock-authorized npm tarballs for one local project.
    InstallDeps {
        /// Project directory containing package.json and package-lock.json.
        #[arg(long, value_name = "PATH")]
        project: PathBuf,
        /// Confirm network acquisition of the lock-authorized package tarballs.
        #[arg(long)]
        yes: bool,
    },
    /// Run one fixed npm project check in Saya's no-egress managed runtime.
    CheckNode {
        /// Project directory containing package.json and package-lock.json.
        #[arg(long, value_name = "PATH")]
        project: PathBuf,
        /// The only supported project script action.
        #[arg(long, value_enum)]
        action: RuntimeCheckAction,
        /// Confirm execution of the selected project script.
        #[arg(long)]
        yes: bool,
    },
}
