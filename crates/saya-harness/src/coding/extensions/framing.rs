use super::ExtensionError;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
pub(super) const MAX_FRAME: usize = 256 * 1024;
#[derive(Clone, Copy)]
pub(super) enum Framing {
    Lines,
    ContentLength,
}

pub(super) async fn write(
    input: &mut ChildStdin,
    framing: Framing,
    value: &Value,
) -> Result<(), ExtensionError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ExtensionError::Protocol)?;
    if bytes.len() > MAX_FRAME {
        return Err(ExtensionError::Limit);
    }
    match framing {
        Framing::Lines => {
            input
                .write_all(&bytes)
                .await
                .map_err(|_| ExtensionError::Process)?;
            input
                .write_all(b"\n")
                .await
                .map_err(|_| ExtensionError::Process)?;
        }
        Framing::ContentLength => {
            input
                .write_all(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes())
                .await
                .map_err(|_| ExtensionError::Process)?;
            input
                .write_all(&bytes)
                .await
                .map_err(|_| ExtensionError::Process)?;
        }
    }
    input.flush().await.map_err(|_| ExtensionError::Process)
}

async fn line(output: &mut BufReader<ChildStdout>, max: usize) -> Result<Vec<u8>, ExtensionError> {
    let mut bytes = Vec::new();
    loop {
        let available = output
            .fill_buf()
            .await
            .map_err(|_| ExtensionError::Process)?;
        if available.is_empty() {
            return Err(ExtensionError::Process);
        }
        let length = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if bytes.len() + length > max {
            return Err(ExtensionError::Limit);
        }
        let complete = available[length - 1] == b'\n';
        bytes.extend_from_slice(&available[..length]);
        output.consume(length);
        if complete {
            return Ok(bytes);
        }
    }
}

pub(super) async fn read(
    output: &mut BufReader<ChildStdout>,
    framing: Framing,
) -> Result<Value, ExtensionError> {
    let bytes = match framing {
        Framing::Lines => line(output, MAX_FRAME).await?,
        Framing::ContentLength => {
            let mut length = None;
            let mut total = 0;
            loop {
                let header = line(output, 1024).await?;
                total += header.len();
                if total > 4096 {
                    return Err(ExtensionError::Limit);
                }
                if header == b"\r\n" {
                    break;
                }
                let text = std::str::from_utf8(&header).map_err(|_| ExtensionError::Protocol)?;
                let (key, value) = text
                    .trim()
                    .split_once(':')
                    .ok_or(ExtensionError::Protocol)?;
                if key.eq_ignore_ascii_case("Content-Length") {
                    if length.is_some() {
                        return Err(ExtensionError::Protocol);
                    }
                    length = Some(
                        value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| ExtensionError::Protocol)?,
                    );
                } else if !key.eq_ignore_ascii_case("Content-Type") {
                    return Err(ExtensionError::Protocol);
                }
            }
            let length = length
                .filter(|length| *length <= MAX_FRAME)
                .ok_or(ExtensionError::Limit)?;
            let mut bytes = vec![0; length];
            output
                .read_exact(&mut bytes)
                .await
                .map_err(|_| ExtensionError::Process)?;
            bytes
        }
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ExtensionError::Protocol)?;
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(ExtensionError::Protocol);
    }
    Ok(value)
}
