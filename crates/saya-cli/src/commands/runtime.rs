use crate::{render::RenderFormat, state_path};
use saya_agent::CancellationToken;
use saya_harness::runtime::{
    NodeInstallConsent, NodeInstallReceipt, install_node as install_node_service,
};
use std::{
    future::Future,
    path::{Path, PathBuf},
};

#[path = "runtime/npm.rs"]
mod npm;
pub(crate) use npm::{check_node, install_deps};

fn completed_operator_result(
    message: String,
    stop: InstallStop,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match stop {
        InstallStop::Completed => crate::commands::result(message, format),
        InstallStop::Interrupt => crate::commands::failure_message(130, message, format),
        #[cfg(unix)]
        InstallStop::Terminate => crate::commands::failure_message(143, message, format),
    }
}

fn stop_code_for(stop: InstallStop, completed_code: i32) -> i32 {
    match stop {
        InstallStop::Completed => completed_code,
        InstallStop::Interrupt => 130,
        #[cfg(unix)]
        InstallStop::Terminate => 143,
    }
}

pub(crate) fn install_node(format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    let state_root = match state_root_from_db_path(&state_path::state_db_path()) {
        Ok(path) => path,
        Err(message) => return crate::commands::failure_message(3, message, format),
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(install_node_cancellable(&state_root, format))
}

pub(super) fn state_root_from_db_path(state_db: &Path) -> Result<PathBuf, String> {
    let rendered = state_db.as_os_str().to_string_lossy();
    let ends_with_separator =
        rendered.ends_with(std::path::MAIN_SEPARATOR) || cfg!(windows) && rendered.ends_with('/');
    if rendered.is_empty() || ends_with_separator || state_db.is_dir() {
        return Err(
            "the configured state database path is empty or names a directory; cannot locate the managed runtime registry"
                .to_owned(),
        );
    }

    match state_db.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.to_path_buf()),
        Some(_) if state_db.file_name().is_some() => Ok(PathBuf::from(".")),
        _ => Err(
            "the configured state database path has no usable parent for the managed runtime registry"
                .to_owned(),
        ),
    }
}

async fn install_node_cancellable(
    state_root: &Path,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let cancellation = CancellationToken::new();
    let work = install_node_service(state_root, Some(NodeInstallConsent::Granted), &cancellation);
    let (outcome, stop) = match settle_install(&cancellation, work).await {
        Ok(settled) => settled,
        Err(error) => {
            return crate::commands::failure_message(
                5,
                format!("could not register runtime stop signals: {error}"),
                format,
            );
        }
    };

    match outcome {
        Ok(receipt) => crate::commands::result(receipt_message(&receipt), format),
        Err(error) => crate::commands::failure_message(
            match stop {
                InstallStop::Completed => 5,
                InstallStop::Interrupt => 130,
                #[cfg(unix)]
                InstallStop::Terminate => 143,
            },
            error.to_string(),
            format,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallStop {
    Completed,
    Interrupt,
    #[cfg(unix)]
    Terminate,
}

#[cfg(unix)]
async fn settle_install<F, T>(
    cancellation: &CancellationToken,
    work: F,
) -> Result<(T, InstallStop), std::io::Error>
where
    F: Future<Output = T>,
{
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    tokio::pin!(work);
    tokio::select! {
        biased;
        signal = &mut interrupt => {
            signal?;
            cancellation.cancel();
            Ok((work.await, InstallStop::Interrupt))
        }
        _ = terminate.recv() => {
            cancellation.cancel();
            Ok((work.await, InstallStop::Terminate))
        }
        result = &mut work => Ok((result, InstallStop::Completed)),
    }
}

#[cfg(not(unix))]
async fn settle_install<F, T>(
    cancellation: &CancellationToken,
    work: F,
) -> Result<(T, InstallStop), std::io::Error>
where
    F: Future<Output = T>,
{
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    tokio::pin!(work);
    tokio::select! {
        biased;
        signal = &mut interrupt => {
            signal?;
            cancellation.cancel();
            Ok((work.await, InstallStop::Interrupt))
        }
        result = &mut work => Ok((result, InstallStop::Completed)),
    }
}

fn receipt_message(receipt: &NodeInstallReceipt) -> String {
    let disposition = match receipt {
        NodeInstallReceipt::Installed { .. } => "Installed",
        NodeInstallReceipt::Reused { .. } => "Reused verified",
    };
    format!(
        "{disposition} managed Node.js runtime: Node {}, npm {}; image manifest {}.",
        receipt.node_version(),
        receipt.npm_version(),
        receipt.image().manifest_digest()
    )
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
