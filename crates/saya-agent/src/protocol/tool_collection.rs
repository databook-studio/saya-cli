use super::streaming::MAX_TOOL_CALLS_PER_RESPONSE;
use crate::{ProviderError, ToolCall};
use std::io::{self, Write};

pub(crate) fn admit_tool_calls(
    collected: &mut Vec<ToolCall>,
    collected_bytes: &mut usize,
    incoming: Vec<ToolCall>,
    byte_limit: usize,
) -> Result<usize, ProviderError> {
    if collected
        .len()
        .checked_add(incoming.len())
        .is_none_or(|count| count > MAX_TOOL_CALLS_PER_RESPONSE)
    {
        return Err(ProviderError::ToolCollectionLimit);
    }
    let mut added = 0usize;
    for call in &incoming {
        let remaining = byte_limit
            .checked_sub(*collected_bytes)
            .and_then(|remaining| remaining.checked_sub(added))
            .ok_or(ProviderError::ToolCollectionLimit)?;
        added = added
            .checked_add(tool_call_bytes(call, remaining)?)
            .ok_or(ProviderError::ToolCollectionLimit)?;
    }
    *collected_bytes = collected_bytes
        .checked_add(added)
        .ok_or(ProviderError::ToolCollectionLimit)?;
    collected.extend(incoming);
    Ok(added)
}

fn tool_call_bytes(call: &ToolCall, limit: usize) -> Result<usize, ProviderError> {
    let head = call
        .id
        .len()
        .checked_add(call.name.len())
        .filter(|bytes| *bytes <= limit)
        .ok_or(ProviderError::ToolCollectionLimit)?;
    let mut counter = ByteCounter { bytes: head, limit };
    serde_json::to_writer(&mut counter, &call.arguments)
        .map_err(|_| ProviderError::ToolCollectionLimit)?;
    Ok(counter.bytes)
}

struct ByteCounter {
    bytes: usize,
    limit: usize,
}

impl Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .checked_add(buffer.len())
            .filter(|bytes| *bytes <= self.limit)
            .ok_or_else(|| io::Error::from(io::ErrorKind::WriteZero))?;
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
