use crate::{ProviderError, ToolCall};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct ToolAssembly {
    calls: BTreeMap<usize, PartialCall>,
    bytes: usize,
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
        if !self.calls.contains_key(&index) && self.calls.len() >= 256 {
            return Err(size_error());
        }
        if let Some(id) = id {
            if let Some(previous) = self.calls.get(&index).and_then(|call| call.id.as_deref()) {
                if previous != id {
                    return Err(ProviderError::InvalidResponse);
                }
            } else if self
                .calls
                .get(&index)
                .and_then(|call| call.id.as_ref())
                .is_none()
            {
                self.reserve(id.len())?;
            }
        }
        if let Some(name) = name {
            self.reserve(name.len())?;
        }
        if let Some(arguments) = arguments {
            self.reserve(arguments.len())?;
        }
        let call = self.calls.entry(index).or_default();
        if let Some(id) = id
            && call.id.is_none()
        {
            call.id = Some(id.into());
        }
        if let Some(name) = name {
            call.name.push_str(name);
        }
        if let Some(arguments) = arguments {
            call.arguments.push_str(arguments);
        }
        Ok(())
    }

    fn reserve(&mut self, bytes: usize) -> Result<(), ProviderError> {
        let next = self.bytes.checked_add(bytes).ok_or_else(size_error)?;
        if next > crate::MAX_STREAM_BYTES {
            return Err(size_error());
        }
        self.bytes = next;
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

    pub(super) fn bytes(&self) -> usize {
        self.bytes
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

fn size_error() -> ProviderError {
    ProviderError::Request("provider stream exceeded size limit".into())
}

#[cfg(test)]
mod tests {
    use super::ToolAssembly;
    use crate::MAX_STREAM_BYTES;

    #[test]
    fn fragmented_tool_arguments_share_the_response_budget() {
        let mut assembly = ToolAssembly::default();
        let fragment = "x".repeat(MAX_STREAM_BYTES / 2);
        assembly
            .push(0, None, Some("tool"), Some(&fragment))
            .unwrap();
        let error = assembly.push(0, None, None, Some(&fragment)).unwrap_err();
        assert!(error.to_string().contains("size limit"));
    }
}
