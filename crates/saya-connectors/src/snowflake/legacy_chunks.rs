use saya_types::{ConnectionError, QueryResult};
use serde_json::Value;
use tokio::time::timeout;

use super::{client::SnowflakeConnector, errors, result};

pub(crate) async fn collect(
    connector: &SnowflakeConnector,
    value: Value,
    max: usize,
    original: String,
) -> Result<QueryResult, ConnectionError> {
    let mut output = result::result(&value, max.saturating_add(1), original.clone())?;
    let data = value.get("data").unwrap_or(&value);
    let headers = data.get("chunkHeaders").and_then(Value::as_object);
    let chunks = data
        .get("chunks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for chunk in chunks {
        if output.rows.len() > max {
            break;
        }
        let url = chunk
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(errors::query)?;
        let mut request = connector.client.get(url);
        for name in [
            "x-amz-server-side-encryption-customer-key",
            "x-amz-server-side-encryption-customer-key-md5",
        ] {
            if let Some(value) = headers
                .and_then(|item| item.get(name))
                .and_then(Value::as_str)
            {
                request = request.header(name, value);
            }
        }
        let response = timeout(connector.timeout, request.send())
            .await
            .map_err(|_| errors::query())?
            .map_err(|_| errors::query())?;
        if !response.status().is_success() {
            return Err(errors::query());
        }
        output.rows.extend(result::chunk_rows(
            &response.text().await.map_err(|_| errors::query())?,
        )?);
    }
    result::bounded(output.columns, output.rows, max, original)
}
