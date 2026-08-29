use serde_json::Value;

pub(crate) fn same_origin(origin: &str, location: &str) -> Option<String> {
    let origin = url::Url::parse(origin).ok()?;
    let value = origin.join(location).ok()?;
    (value.scheme() == origin.scheme()
        && value.host_str() == origin.host_str()
        && value.port_or_known_default() == origin.port_or_known_default()
        && value.username().is_empty()
        && value.password().is_none()
        && value.fragment().is_none())
    .then(|| value.into())
}

pub(crate) fn handle(value: &Value) -> Option<String> {
    value
        .get("statementHandle")
        .or_else(|| value.get("data").and_then(|d| d.get("statementHandle")))
        .and_then(Value::as_str)
        .and_then(|item| uuid::Uuid::parse_str(item).ok().map(|_| item.to_owned()))
}

/// Validates a chunk-download URL taken from an API response before it is
/// handed to the authenticated client. Accepted: same-origin relative or
/// absolute URLs (the common deployment — chunks served by the account host)
/// and https URLs on `*.snowflakecomputing.com`. Everything else — plaintext
/// off-origin, lookalike hosts, embedded credentials, fragments — is
/// rejected, so a misdirected or hostile chunk location cannot turn the
/// client into a retrieval oracle.
pub(crate) fn download_url(origin: &str, raw: &str) -> Option<String> {
    if let Some(same) = same_origin(origin, raw) {
        return Some(same);
    }
    let value = url::Url::parse(raw).ok()?;
    (value.scheme() == "https"
        && value.host_str()?.ends_with(".snowflakecomputing.com")
        && value.username().is_empty()
        && value.password().is_none()
        && value.fragment().is_none())
    .then(|| value.into())
}

#[cfg(test)]
mod tests {
    use super::{download_url, handle, same_origin};
    use serde_json::json;

    #[test]
    fn accepts_only_same_origin_status_urls_and_uuid_handles() {
        assert_eq!(
            same_origin("https://good.example", "/status"),
            Some("https://good.example/status".into())
        );
        assert!(same_origin("https://good.example", "https://good.example.evil/status").is_none());
        assert!(same_origin("https://good.example", "https://user@good.example/status").is_none());
        assert_eq!(
            handle(&json!({"statementHandle":"00000000-0000-4000-8000-000000000001"})).as_deref(),
            Some("00000000-0000-4000-8000-000000000001")
        );
        assert!(handle(&json!({"statementHandle":"not-a-uuid"})).is_none());
    }

    #[test]
    fn accepts_only_snowflake_or_same_origin_chunk_urls() {
        const ORIGIN: &str = "https://acme.snowflakecomputing.com";
        assert_eq!(
            download_url(ORIGIN, "/queries/x/chunks/0?s=1"),
            Some(format!("{ORIGIN}/queries/x/chunks/0?s=1"))
        );
        assert_eq!(
            download_url(ORIGIN, "https://acme.snowflakecomputing.com/c"),
            Some("https://acme.snowflakecomputing.com/c".into())
        );
        // Lookalike hosts, plaintext off-origin, credentials, fragments: rejected.
        assert!(download_url(ORIGIN, "https://acme.snowflakecomputing.com.evil/c").is_none());
        assert!(download_url(ORIGIN, "http://other.snowflakecomputing.com/c").is_none());
        assert!(download_url(ORIGIN, "https://tok@acme.snowflakecomputing.com/c").is_none());
        assert!(download_url(ORIGIN, "https://acme.snowflakecomputing.com/c#frag").is_none());
        assert!(download_url(ORIGIN, "https://storage.amazonaws.com/bucket/c").is_none());
    }
}
