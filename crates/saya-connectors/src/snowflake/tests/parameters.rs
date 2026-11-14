use super::*;

fn bound(name: &str, value: saya_types::ParamValue) -> saya_types::BoundParam {
    saya_types::BoundParam {
        name: name.to_owned(),
        value,
    }
}

/// A keypair connector binds `:name` placeholders natively: the v2 statement
/// request carries the `?`-rewritten SQL beside the 1-based bindings object,
/// and the original SQL and values never swap places.
#[tokio::test]
async fn keypair_query_binds_parameters_into_the_v2_request() {
    let (origin, seen) = server(vec![Reply::json(v2_done(&[1], 0))]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let request = saya_types::QueryRequest::with_params(
        "SELECT 1 WHERE 1 = :v AND :since < 1".to_owned(),
        2,
        vec![
            bound("v", saya_types::ParamValue::Integer(12)),
            bound(
                "since",
                saya_types::ParamValue::Date("2024-02-29".to_owned()),
            ),
        ],
    );
    let output = item.execute(request).await.unwrap();
    assert_eq!(output.rows, rows(&[1]));
    let posted: Value = serde_json::from_str(request_body(&seen.lock().await[0])).unwrap();
    assert!(
        posted["statement"]
            .as_str()
            .unwrap()
            .contains("SELECT 1 WHERE 1 = ? AND ? < 1"),
        "markers must be rewritten in marker order: {posted:?}"
    );
    assert!(posted["statement"].as_str().unwrap().contains("LIMIT 3"));
    assert!(
        !posted["statement"].as_str().unwrap().contains(":v"),
        "a :name leaked: {posted:?}"
    );
    assert!(
        !posted["statement"].as_str().unwrap().contains(":since"),
        "a :name leaked: {posted:?}"
    );
    assert_eq!(
        posted["bindings"]["1"],
        json!({"type": "FIXED", "value": "12"})
    );
    assert_eq!(
        posted["bindings"]["2"],
        json!({"type": "DATE", "value": "1709164800000"})
    );
    assert!(posted["bindings"].get("3").is_none());
}

/// The legacy and browser paths have no bindings field on the wire, so a
/// parameterized request refuses as unsupported before any network call.
#[tokio::test]
async fn legacy_paths_refuse_parameters_before_any_network_call() {
    let (origin, seen) = server(vec![]).await;
    for auth in [
        userpass(),
        Auth::ExternalBrowser(ExternalBrowser {
            enabled: false,
            token: Arc::new(Mutex::new(None)),
        }),
    ] {
        let mut item = connector(auth);
        item.origin = origin.clone();
        let request = saya_types::QueryRequest::with_params(
            "SELECT 1 WHERE 1 = :v".to_owned(),
            1,
            vec![bound("v", saya_types::ParamValue::Integer(1))],
        );
        let error = item.execute(request).await.unwrap_err();
        assert!(
            matches!(error, saya_types::ConnectionError::Unsupported(_)),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "unsupported operation: parameters need keypair auth on Snowflake"
        );
    }
    assert!(
        seen.lock().await.is_empty(),
        "a refusal must not touch the network"
    );
}

/// Parameters ride only keypair auth, so the capability tracks the auth path.
#[test]
fn parameter_capability_tracks_the_auth_path() {
    assert!(connector(Auth::Keypair(keypair())).supports_parameters());
    assert!(!connector(userpass()).supports_parameters());
    assert!(
        !connector(Auth::ExternalBrowser(ExternalBrowser {
            enabled: false,
            token: Arc::new(Mutex::new(None)),
        }))
        .supports_parameters()
    );
}
