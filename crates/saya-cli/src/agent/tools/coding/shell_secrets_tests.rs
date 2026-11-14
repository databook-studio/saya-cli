//! What the host policy names: every environment-variable name the resolved
//! config references as a secret, saya's own `SAYA_` namespace, and the
//! values already resolved for those names — and nothing read to get them.

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::host_policy;
use crate::{GlobalOptions, load_with_sources};

/// A private user-config directory holding `config.toml`, removed on drop.
struct ConfigDir(PathBuf);

impl ConfigDir {
    fn new(tag: &str, config: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "saya-shell-secrets-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("the config directory must be creatable");
        if !config.is_empty() {
            fs::write(path.join("config.toml"), config).expect("the config file must be writable");
        }
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ConfigDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A composed runtime over `config`, with `process` as this process's
/// environment as the loader sees it.
fn runtime(tag: &str, config: &str, process: &[(&str, &str)]) -> crate::RuntimeConfig {
    let dir = ConfigDir::new(tag, config);
    let cwd = ConfigDir::new(&format!("{tag}-cwd"), "");
    let process = process
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let runtime = load_with_sources(&GlobalOptions::default(), cwd.path(), dir.path(), process)
        .expect("the fixture config resolves");
    // The fixture directory is removed with the runtime still in hand: only
    // the composed values matter here, and none of them re-reads the disk.
    drop(dir);
    drop(cwd);
    runtime
}

/// A configured provider key and a configured database password are both
/// removed by name, and the value saya already holds is named for scrubbing.
#[test]
fn configured_secret_references_are_removed_and_their_values_scrubbed() {
    let runtime = runtime(
        "configured",
        "[ai]\napi_key = { env = \"SAYA_TEST_PROVIDER_KEY\" }\n",
        &[("SAYA_TEST_PROVIDER_KEY", "sk-planted-provider")],
    );
    let policy = host_policy(&runtime);
    assert!(
        policy
            .remove_env
            .contains(&"SAYA_TEST_PROVIDER_KEY".to_owned()),
        "a configured provider key is removed by name: {:?}",
        policy.remove_env
    );
    assert!(
        policy
            .secret_values
            .contains(&"sk-planted-provider".to_owned()),
        "the value saya already holds is scrubbed from the capture: {:?}",
        policy.secret_values
    );
}

/// A database profile's password reference is a secret name too, whether it
/// sits in the connections file or the selected profile.
#[test]
fn a_configured_database_password_reference_is_removed() {
    let dir = ConfigDir::new("connections", "");
    let cwd = ConfigDir::new("connections-cwd", "");
    fs::write(
        dir.path().join("connections.toml"),
        "[profiles.warehouse]\ntype = \"postgresql\"\nhost = \"localhost\"\ndatabase = \"db\"\n\
         user = \"u\"\npassword = { env = \"SAYA_TEST_DB_PASSWORD\" }\n",
    )
    .expect("the connections file must be writable");
    let runtime = load_with_sources(
        &GlobalOptions::default(),
        cwd.path(),
        dir.path(),
        BTreeMap::from([("SAYA_TEST_DB_PASSWORD".to_owned(), "db-planted".to_owned())]),
    )
    .expect("the connections fixture resolves");
    let policy = host_policy(&runtime);
    assert!(
        policy
            .remove_env
            .contains(&"SAYA_TEST_DB_PASSWORD".to_owned()),
        "a database password reference is removed by name: {:?}",
        policy.remove_env
    );
    assert!(policy.secret_values.contains(&"db-planted".to_owned()));
    drop(dir);
    drop(cwd);
}

/// Saya's own namespace goes wholesale: `SAYA_API_KEY` whether or not it is
/// configured, and every other `SAYA_` name this process already carries.
#[test]
fn the_saya_namespace_is_removed_wholesale() {
    // The names this process really carries are the ones that matter, so the
    // assertion reads the same environment the policy does.
    let planted = std::env::vars_os()
        .filter_map(|(name, _)| name.to_str().map(str::to_owned))
        .filter(|name| name.starts_with("SAYA_") && name != "SAYA_API_KEY")
        .collect::<Vec<_>>();
    let runtime = runtime("namespace", "", &[]);
    let policy = host_policy(&runtime);
    assert!(
        policy.remove_env.contains(&"SAYA_API_KEY".to_owned()),
        "SAYA_API_KEY is always removed: {:?}",
        policy.remove_env
    );
    for name in planted {
        assert!(
            policy.remove_env.contains(&name),
            "{name} starts with SAYA_ and must be removed"
        );
    }
}

/// Nothing new is read to build the policy: a file-backed reference has no
/// value in hand, so no value is named for scrubbing — the file is never
/// opened here.
#[test]
fn a_file_backed_reference_contributes_no_value() {
    let secret_file = std::env::temp_dir().join(format!(
        "saya-shell-secrets-file-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&secret_file, "file-backed-planted").expect("the secret file must be writable");
    let runtime = runtime(
        "file-backed",
        &format!(
            "[ai]\napi_key = {{ file = \"{}\" }}\n",
            secret_file.display()
        ),
        &[],
    );
    let policy = host_policy(&runtime);
    assert!(
        !policy
            .secret_values
            .contains(&"file-backed-planted".to_owned()),
        "building a shell must not open a secret file: {:?}",
        policy.secret_values
    );
    let _ = fs::remove_file(secret_file);
}
