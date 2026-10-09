//! One AI endpoint mirrored for display, shared by the file and resolved
//! views.

use serde::Serialize;

/// One AI endpoint mirrored for display. `Option` means "not declared" in
/// the file view and "resolved to none" in the resolved view; either way a
/// secret appears only as its redacted reference label, never as a value —
/// the same discipline as `api_key_reference` on the parent views.
#[derive(Debug, Clone, Serialize)]
pub struct EndpointDiagnostics {
    pub name: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key_reference: Option<String>,
}
