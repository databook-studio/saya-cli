use crate::{ProviderError, ToolCall};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct ToolAssembly {
    calls: BTreeMap<usize, PartialCall>,
}

#[derive(Default)]
struct PartialCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

impl ToolAssembly {
    pub(super) fn push(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) -> Result<(), ProviderError> {
        let call = self.calls.entry(index).or_default();
        if let Some(id) = id {
            if let Some(previous) = &call.id {
                if previous != id {
                    return Err(ProviderError::InvalidResponse);
                }
            } else {
                call.id = Some(id.into());
            }
        }
        if let Some(name) = name {
            call.name.push_str(name);
        }
        if let Some(arguments) = arguments {
            call.arguments.push_str(arguments);
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Vec<ToolCall>, ProviderError> {
        self.calls
            .into_iter()
            .map(|(index, call)| {
                let arguments: serde_json::Value = serde_json::from_str(&call.arguments)
                    .map_err(|_| ProviderError::InvalidResponse)?;
                if call.name.is_empty() || !arguments.is_object() {
                    return Err(ProviderError::InvalidResponse);
                }
                Ok(ToolCall {
                    id: call.id.unwrap_or_else(|| format!("call-{index}")),
                    name: call.name,
                    arguments,
                })
            })
            .collect()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// Raw argument fragments assembled so far, one per partial call, for the
    /// truncation signal. A capped tool call never parses, so the fragments
    /// ride the error for diagnosis rather than becoming calls.
    pub(super) fn partial_json(&self) -> Vec<String> {
        self.calls
            .values()
            .map(|call| call.arguments.clone())
            .collect()
    }
}
