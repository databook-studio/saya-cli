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

#[cfg(test)]
mod tests {
    use super::{handle, same_origin};
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
}
