use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::{DatabaseProfile, SecretRef, SnowflakeAuth};

#[tokio::test]
async fn snowflake_live_contract_is_opt_in() {
    let Ok(account) = std::env::var("SAYA_TEST_SNOWFLAKE_ACCOUNT") else {
        eprintln!("SKIPPED: SAYA_TEST_SNOWFLAKE_ACCOUNT is unset");
        return;
    };
    let Ok(user) = std::env::var("SAYA_TEST_SNOWFLAKE_USER") else {
        eprintln!("SKIPPED: SAYA_TEST_SNOWFLAKE_USER is unset");
        return;
    };
    let Ok(password) = std::env::var("SAYA_TEST_SNOWFLAKE_PASSWORD") else {
        eprintln!("SKIPPED: SAYA_TEST_SNOWFLAKE_PASSWORD is unset");
        return;
    };
    let profile = DatabaseProfile::Snowflake {
        account,
        user,
        auth_type: SnowflakeAuth::Userpass,
        private_key: None,
        password: Some(SecretRef::Env {
            env: "password".into(),
        }),
        passphrase: None,
        warehouse: std::env::var("SAYA_TEST_SNOWFLAKE_WAREHOUSE").ok(),
        database: std::env::var("SAYA_TEST_SNOWFLAKE_DATABASE").ok(),
        schema: std::env::var("SAYA_TEST_SNOWFLAKE_SCHEMA").ok(),
        role: None,
    };
    let resolver = MapSecretResolver::new([(String::from("password"), password)]);
    let connector = build_connector(&profile, &resolver, ConnectorOptions::default())
        .await
        .unwrap();
    connector.connect().await.unwrap();
    connector
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    if profile_database_and_schema(&profile) {
        connector.schema().await.unwrap();
    }
}

fn profile_database_and_schema(profile: &DatabaseProfile) -> bool {
    matches!(
        profile,
        DatabaseProfile::Snowflake {
            database: Some(_),
            schema: Some(_),
            ..
        }
    )
}
