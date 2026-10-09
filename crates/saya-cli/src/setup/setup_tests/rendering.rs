#[test]
fn api_key_is_only_an_env_reference() {
    let dir = temp_dir("env-ref");
    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: None,
    };
    let planned = plan(&dir, &draft).unwrap();
    let content = &planned.writes[0].content;
    assert!(content.contains("api_key = { env = \"OPENAI_API_KEY\" }"));
    assert!(!content.contains("api_key = \""), "never an inline value");
    let parsed = ConfigFile::from_toml(content).unwrap();
    assert_eq!(
        parsed.ai.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".into()
        })
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rendered_files_parse_with_real_parsers() {
    let cases = vec![
        (
            "pg",
            DatabaseProfile::Postgres {
                host: "db.example.com".into(),
                port: Some(5432),
                database: "app".into(),
                user: "reader".into(),
                ssl_mode: Some(saya_types::PostgresSslMode::Require),
                password: Some(SecretRef::Env {
                    env: "SAYA_PG_PASSWORD".into(),
                }),
            },
        ),
        (
            "mysql",
            DatabaseProfile::Mysql {
                host: "db.example.com".into(),
                port: None,
                database: "app".into(),
                user: "reader".into(),
                ssl_mode: Some(saya_types::MySqlSslMode::VerifyIdentity),
                ssl_ca: None,
                password: Some(SecretRef::Env {
                    env: "SAYA_MYSQL_PASSWORD".into(),
                }),
            },
        ),
        (
            "sqlite",
            DatabaseProfile::Sqlite {
                path: "/tmp/t.db".into(),
                read_only: true,
            },
        ),
        (
            "duckdb",
            DatabaseProfile::DuckDb {
                path: "/tmp/t.duckdb".into(),
                read_only: Some(false),
            },
        ),
    ];
    for (name, profile) in cases {
        let dir = temp_dir(&format!("render-{name}"));
        let draft = SetupDraft {
            provider: None,
            profile: Some(ProfileDraft {
                name: name.into(),
                profile: profile.clone(),
            }),
        };
        let planned = plan(&dir, &draft).unwrap();
        let content = &planned.writes[0].content;
        let parsed =
            ConnectionsFile::from_toml(content).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            parsed.profiles.get(name),
            Some(&profile),
            "round-trips the typed profile"
        );
        assert!(content.starts_with(&format!("[profiles.{name}]")));
        let _ = fs::remove_dir_all(dir);
    }

    // A dotted profile name must survive the round trip (quoted header).
    let dir = temp_dir("render-dotted");
    let draft = SetupDraft {
        provider: None,
        profile: Some(profile_draft("team.laptop")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let parsed = ConnectionsFile::from_toml(&planned.writes[0].content).unwrap();
    assert!(parsed.profiles.contains_key("team.laptop"));
    let _ = fs::remove_dir_all(dir);
}

/// The warehouse engines render through the same serializer and must round
/// trip with the real parser, for every Snowflake auth variant included.
/// Optional fields left unset are omitted from the TOML and parse back as
/// `None`.
#[test]
fn warehouse_profiles_render_and_round_trip() {
    let cases = vec![
        (
            "snowflake_keypair",
            DatabaseProfile::Snowflake {
                account: "org-account.us-east-1.aws".into(),
                user: "jane".into(),
                auth_type: saya_types::SnowflakeAuth::Keypair,
                private_key: Some(SecretRef::File {
                    file: "/absolute/path/to/rsa_key.p8".into(),
                }),
                password: None,
                passphrase: Some(SecretRef::Env {
                    env: "SAYA_SNOWFLAKE_PASSPHRASE".into(),
                }),
                warehouse: Some("ANALYTICS".into()),
                database: Some("PROD".into()),
                schema: Some("PUBLIC".into()),
                role: Some("ANALYST".into()),
            },
        ),
        (
            "snowflake_userpass",
            DatabaseProfile::Snowflake {
                account: "org-account.us-east-1.aws".into(),
                user: "jane".into(),
                auth_type: saya_types::SnowflakeAuth::Userpass,
                private_key: None,
                password: Some(SecretRef::Env {
                    env: "SAYA_SNOWFLAKE_PASSWORD".into(),
                }),
                passphrase: None,
                warehouse: None,
                database: None,
                schema: None,
                role: None,
            },
        ),
        (
            "snowflake_browser",
            DatabaseProfile::Snowflake {
                account: "org-account.us-east-1.aws".into(),
                user: "jane".into(),
                auth_type: saya_types::SnowflakeAuth::Externalbrowser,
                private_key: None,
                password: None,
                passphrase: None,
                warehouse: None,
                database: None,
                schema: None,
                role: None,
            },
        ),
        (
            "clickhouse",
            DatabaseProfile::ClickHouse {
                host: "clickhouse.internal.example".into(),
                port: Some(8443),
                database: Some("warehouse".into()),
                user: Some("saya_readonly".into()),
                password: Some(SecretRef::Env {
                    env: "SAYA_CLICKHOUSE_PASSWORD".into(),
                }),
                secure: Some(true),
            },
        ),
        (
            "bigquery",
            DatabaseProfile::BigQuery {
                project: "my-gcp-project".into(),
                dataset: Some("warehouse".into()),
                location: None,
                max_bytes_billed: Some(1_000_000_000_000),
                service_account_key: SecretRef::File {
                    file: "/absolute/path/to/service-account.json".into(),
                },
            },
        ),
    ];
    for (name, profile) in cases {
        let dir = temp_dir(&format!("render-warehouse-{name}"));
        let draft = SetupDraft {
            provider: None,
            profile: Some(ProfileDraft {
                name: name.into(),
                profile: profile.clone(),
            }),
        };
        let planned = plan(&dir, &draft).unwrap_or_else(|error| panic!("{name}: {error}"));
        let content = &planned.writes[0].content;
        let parsed =
            ConnectionsFile::from_toml(content).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            parsed.profiles.get(name),
            Some(&profile),
            "{name} round-trips the typed profile"
        );
        assert!(content.starts_with(&format!("[profiles.{name}]")));
        assert!(
            !content.contains("password = \"")
                && !content.contains("private_key = \"")
                && !content.contains("passphrase = \"")
                && !content.contains("service_account_key = \""),
            "{name} never renders a secret as an inline value: {content}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
