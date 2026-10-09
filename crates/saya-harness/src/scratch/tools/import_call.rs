use saya_agent::CancellationToken;

use crate::scratch::import::{MAX_IMPORT_FILE_BYTES, import_bytes};
use crate::scratch::{ImportError, ImportResult, ScratchError};

use super::ScratchSql;

impl ScratchSql {
    pub(super) async fn import(
        &self,
        arguments: &serde_json::Map<String, serde_json::Value>,
        cancellation: &CancellationToken,
    ) -> Result<ImportResult, ScratchError> {
        // A pre-cancelled turn refuses before the workspace file is read.
        if cancellation.is_cancelled() {
            return Err(ImportError::Cancelled.into());
        }
        if arguments.keys().any(|key| {
            !matches!(
                key.as_str(),
                "path" | "table" | "header" | "delimiter" | "if_exists"
            )
        }) {
            return Err(ImportError::InvalidArguments.into());
        }
        let workspace = self
            .workspace
            .as_ref()
            .ok_or(ImportError::WorkspaceUnavailable)?;
        let path = required_string(arguments, "path")?;
        let table = required_string(arguments, "table")?;
        let header = optional_bool(arguments, "header", true)?;
        let delimiter = optional_delimiter(arguments)?;
        let replace = match arguments.get("if_exists") {
            None => false,
            Some(serde_json::Value::String(value)) => match value.as_str() {
                "fail" => false,
                "replace" => true,
                _ => return Err(ImportError::InvalidIfExists.into()),
            },
            Some(_) => return Err(ImportError::InvalidArguments.into()),
        };
        let file = workspace
            .read_for_scratch_import(path)
            .map_err(ImportError::from)?;
        if file.bytes.len() > MAX_IMPORT_FILE_BYTES {
            return Err(ImportError::FileTooLarge {
                max: MAX_IMPORT_FILE_BYTES,
            }
            .into());
        }
        let db = self.db.clone();
        let bytes = file.bytes;
        let table = table.to_owned();
        let cancellation = cancellation.clone();
        tokio::task::spawn_blocking(move || {
            import_bytes(
                &db,
                &table,
                &bytes,
                header,
                delimiter,
                replace,
                &cancellation,
            )
        })
        .await
        .map_err(|_| ScratchError::Import(ImportError::Database))?
        .map_err(ScratchError::Import)
    }
}

fn required_string<'a>(
    arguments: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, ScratchError> {
    arguments
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or(ScratchError::Import(ImportError::InvalidArguments))
}

fn optional_bool(
    arguments: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, ScratchError> {
    match arguments.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or(ScratchError::Import(ImportError::InvalidArguments)),
    }
}

fn optional_delimiter(
    arguments: &serde_json::Map<String, serde_json::Value>,
) -> Result<u8, ScratchError> {
    let value = match arguments.get("delimiter") {
        None => ",",
        Some(serde_json::Value::String(value)) => value,
        Some(_) => return Err(ImportError::InvalidArguments.into()),
    };
    if value.len() == 1 && value.is_ascii() {
        Ok(value.as_bytes()[0])
    } else {
        Err(ImportError::InvalidDelimiter.into())
    }
}
