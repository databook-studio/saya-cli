use super::super::ExtensionError;
use serde_json::Value;
#[derive(Clone, PartialEq, Eq)]
pub struct LspDiagnostic {
    pub line: u32,
    pub character: u32,
    pub severity: Option<u8>,
    pub message: String,
}
#[derive(Clone, PartialEq, Eq)]
pub struct DocumentDiagnostics {
    pub document_uri: String,
    pub version: i64,
    pub diagnostics: Vec<LspDiagnostic>,
}
pub(super) fn decode(
    params: &Value,
    uri: &str,
    version: i64,
) -> Result<DocumentDiagnostics, ExtensionError> {
    if params.get("uri").and_then(Value::as_str) != Some(uri)
        || params.get("version").and_then(Value::as_i64) != Some(version)
    {
        return Err(ExtensionError::Protocol);
    }
    let items = params
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or(ExtensionError::Protocol)?;
    if items.len() > 256 {
        return Err(ExtensionError::Limit);
    }
    let mut diagnostics = Vec::with_capacity(items.len());
    for item in items {
        let position = &item["range"]["start"];
        let line = position["line"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(ExtensionError::Protocol)?;
        let character = position["character"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(ExtensionError::Protocol)?;
        let message = item["message"]
            .as_str()
            .filter(|text| text.len() <= 8192)
            .ok_or(ExtensionError::Limit)?
            .to_owned();
        let severity = match item.get("severity") {
            None => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .filter(|n| (1..=4).contains(n))
                    .ok_or(ExtensionError::Protocol)? as u8,
            ),
        };
        diagnostics.push(LspDiagnostic {
            line,
            character,
            severity,
            message,
        });
    }
    Ok(DocumentDiagnostics {
        document_uri: uri.into(),
        version,
        diagnostics,
    })
}

impl std::fmt::Debug for LspDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspDiagnostic")
            .field("line", &self.line)
            .field("character", &self.character)
            .field("severity", &self.severity)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for DocumentDiagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentDiagnostics")
            .field("version", &self.version)
            .field("count", &self.diagnostics.len())
            .finish_non_exhaustive()
    }
}
