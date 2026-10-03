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
    pub(super) fn start(
        &mut self,
        index: usize,
        id: &str,
        name: &str,
    ) -> Result<(), ProviderError> {
        if self.calls.contains_key(&index) {
            return Err(ProviderError::InvalidResponse);
        }
        if self.calls.len() >= crate::MAX_TOOL_CALLS_PER_RESPONSE {
            return Err(ProviderError::ToolCollectionLimit);
        }
        self.reserve(
            id.len()
                .checked_add(name.len())
                .ok_or(ProviderError::ToolCollectionLimit)?,
        )?;
        self.calls.insert(
            index,
            PartialCall {
                id: Some(id.into()),
                name: name.into(),
                arguments: String::new(),
            },
        );
        Ok(())
    }

    pub(super) fn append_arguments(
        &mut self,
        index: usize,
        arguments: &str,
    ) -> Result<(), ProviderError> {
        if !self.calls.contains_key(&index) {
            return Err(ProviderError::InvalidResponse);
        }
        self.reserve(arguments.len())?;
        if let Some(call) = self.calls.get_mut(&index) {
            call.arguments.push_str(arguments);
        }
        Ok(())
    }

    pub(super) fn push(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) -> Result<(), ProviderError> {
        if !self.calls.contains_key(&index)
            && self.calls.len() >= crate::MAX_TOOL_CALLS_PER_RESPONSE
        {
            return Err(ProviderError::ToolCollectionLimit);
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
        let next = self
            .bytes
            .checked_add(bytes)
            .ok_or(ProviderError::ToolCollectionLimit)?;
        if next > crate::MAX_STREAM_BYTES {
            return Err(ProviderError::ToolCollectionLimit);
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

    pub(super) fn finish_anthropic(self) -> Result<Vec<ToolCall>, ProviderError> {
        self.calls
            .into_iter()
            .map(|(index, call)| {
                let arguments = if call.arguments.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&call.arguments)
                        .map_err(|_| ProviderError::InvalidResponse)?
                };
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
        assert!(matches!(error, crate::ProviderError::ToolCollectionLimit));
    }
}
