use saya_types::{ConnectionError, QueryResult};
use serde_json::Value;
use tokio::time::timeout;

use super::{client::SnowflakeConnector, errors, result, status_url};

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
        // The URL comes from the API response; validate it before handing it
        // to the authenticated client.
        let url = chunk
            .get("url")
            .and_then(Value::as_str)
            .and_then(|raw| status_url::download_url(&connector.origin, raw))
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
        // Buffer the body under the same deadline and byte budget as the
        // rest of the result pipeline instead of reading unbounded text.
        let body = timeout(connector.timeout, response.bytes())
            .await
            .map_err(|_| errors::query())?
            .map_err(|_| errors::query())?;
        if body.len() > crate::common::MAX_RESULT_BYTES {
            return Err(errors::query());
        }
        let text = std::str::from_utf8(&body).map_err(|_| errors::query())?;
        output.rows.extend(result::chunk_rows(text)?);
    }
    result::bounded(output.columns, output.rows, max, original)
}
