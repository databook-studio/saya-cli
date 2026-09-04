use saya_types::{DatabaseProfile, ProfileIdentity};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn profile_identity(name: &str, profile: &DatabaseProfile, scope: &Path) -> ProfileIdentity {
    let mut hash = Sha256::new();
    field(&mut hash, name);
    field_bytes(&mut hash, scope.as_os_str().as_encoded_bytes());
    match profile {
        DatabaseProfile::Postgres {
            host,
            port,
            database,
            user,
            ssl_mode,
            ..
        } => fields(
            &mut hash,
            [
                "postgres",
                host,
                database,
                user,
                &format!("{port:?}"),
                &format!("{ssl_mode:?}"),
            ],
        ),
        DatabaseProfile::Mysql {
            host,
            port,
            database,
            user,
            ssl_mode,
            ssl_ca,
            ..
        } => fields(
            &mut hash,
            [
                "mysql",
                host,
                database,
                user,
                &format!("{port:?}"),
                &format!("{ssl_mode:?}"),
                &format!("{}", ssl_ca.is_some()),
            ],
        ),
        DatabaseProfile::DuckDb { path, read_only } => {
            fields(&mut hash, ["duckdb", path, &format!("{read_only:?}")]);
        }
        DatabaseProfile::Sqlite { path, read_only } => {
            fields(&mut hash, ["sqlite", path, &format!("{read_only:?}")]);
        }
        DatabaseProfile::Snowflake {
            account,
            user,
            auth_type,
            warehouse,
            database,
            schema,
            role,
            ..
        } => fields(
            &mut hash,
            [
                "snowflake",
                account,
                user,
                &format!("{auth_type:?}"),
                &format!("{warehouse:?}"),
                &format!("{database:?}"),
                &format!("{schema:?}"),
                &format!("{role:?}"),
            ],
        ),
        DatabaseProfile::ClickHouse {
            host,
            port,
            database,
            user,
            secure,
            ..
        } => fields(
            &mut hash,
            [
                "clickhouse",
                host,
                &format!("{database:?}"),
                &format!("{user:?}"),
                &format!("{port:?}"),
                &format!("{secure:?}"),
            ],
        ),
    }
    let digest = hash.finalize();
    let mut value = String::from("p-");
    for byte in digest {
        value.push_str(&format!("{byte:02x}"));
    }
    ProfileIdentity::parse(&value).expect("profile identity is constructed to the validated shape")
}

fn fields<'a>(hash: &mut Sha256, values: impl IntoIterator<Item = &'a str>) {
    for value in values {
        field(hash, value);
    }
}

fn field(hash: &mut Sha256, value: &str) {
    field_bytes(hash, value.as_bytes());
}

fn field_bytes(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

#[cfg(test)]
mod tests {
    use saya_store::{SchemaStore, SqliteStateStore};
    use saya_types::{DatabaseProfile, ProfileIdentity};
    use std::path::Path;

    #[test]
    fn identity_is_stable_and_opaque() {
        let value = super::profile_identity(
            "quoted profile / password",
            &profile("one.duckdb"),
            Path::new("/project-one/connections.toml"),
        );
        assert_eq!(value.as_str().len(), 66);
        assert!(!value.as_str().contains("password"));
    }

    #[test]
    fn derived_identity_parses_as_a_profile_identity() {
        let value = super::profile_identity(
            "parses",
            &profile("parses.duckdb"),
            Path::new("/project-parses/connections.toml"),
        );
        assert_eq!(
            ProfileIdentity::parse(value.as_str()).unwrap().as_str(),
            value.as_str()
        );
    }

    #[tokio::test]
    async fn same_relative_profile_in_different_scopes_cannot_share_a_schema_fallback() {
        let profile = profile("./data.duckdb");
        let first = super::profile_identity(
            "analytics",
            &profile,
            Path::new("/project-one/connections.toml"),
        );
        let second = super::profile_identity(
            "analytics",
            &profile,
            Path::new("/project-two/connections.toml"),
        );
        assert_ne!(first, second);
        let root = std::env::temp_dir().join(format!("saya-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        store
            .upsert_schema(first.as_str(), &saya_types::SchemaTree::default())
            .await
            .unwrap();
        assert!(store.get_schema(second.as_str()).await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    fn profile(path: &str) -> DatabaseProfile {
        DatabaseProfile::DuckDb {
            path: path.into(),
            read_only: Some(true),
        }
    }
}
