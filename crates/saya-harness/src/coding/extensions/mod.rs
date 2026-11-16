//! Configured local protocol servers, admitted only through proven confinement.
mod config;
mod framing;
pub mod lsp;
pub mod mcp;
mod process;
pub(crate) use config::exchange_with_cleanup;
pub use config::{ExtensionConfig, ExtensionError};

mod receipt;
pub use receipt::{ExtensionProcessEvent, ExtensionProcessObserver, ExtensionProcessReceipt};
