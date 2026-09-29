//! The Snowflake question set: the auth choice (keypair recommended, browser
//! SSO, password via env) and only the fields that auth needs. Secrets are
//! taken as an environment-variable name or a key file path — never a value.

use saya_types::{DatabaseProfile, SecretRef, SnowflakeAuth};

use super::draft::validate_env_name;
use super::prompt::{Cancel, Prompter, checked, text_field};
use super::prompt_database::env_ref;
use super::prompt_warehouse::{optional, required_file};

/// An env-var name that must be given (userpass cannot work without it).
fn required_env(line: &str) -> Result<String, String> {
    match line.trim() {
        "" => Err("this field is required".to_owned()),
        trimmed => checked(trimmed, validate_env_name),
    }
}

pub(crate) fn snowflake(prompter: &mut Prompter<'_>) -> Result<DatabaseProfile, Cancel> {
    let account = prompter.ask(
        "Snowflake account (e.g. org-account.us-east-1.aws): ",
        |l| text_field(l, None),
    )?;
    let user = prompter.ask("User: ", |l| text_field(l, None))?;
    let (auth_type, private_key, password, passphrase) = match prompter.menu(
        "Snowflake authentication:",
        &[
            "keypair (recommended)",
            "externalbrowser (browser SSO)",
            "password (environment variable name)",
        ],
        Some(0),
    )? {
        0 => {
            let private_key = prompter.ask(
                "Private key file path (PKCS#8, e.g. /keys/rsa_key.p8): ",
                required_file,
            )?;
            let passphrase = prompter.ask(
                "Passphrase environment-variable name (blank to skip): ",
                env_ref,
            )?;
            (SnowflakeAuth::Keypair, Some(private_key), None, passphrase)
        }
        1 => (SnowflakeAuth::Externalbrowser, None, None, None),
        _ => {
            let password = prompter.ask(
                "Password environment-variable name (required): ",
                required_env,
            )?;
            (
                SnowflakeAuth::Userpass,
                None,
                Some(SecretRef::Env { env: password }),
                None,
            )
        }
    };
    let warehouse = prompter.ask("Warehouse (blank to skip): ", optional)?;
    let database = prompter.ask("Database (blank to skip): ", optional)?;
    let schema = prompter.ask("Schema (blank to skip): ", optional)?;
    let role = prompter.ask("Role (blank to skip): ", optional)?;
    Ok(DatabaseProfile::Snowflake {
        account,
        user,
        auth_type,
        private_key,
        password,
        passphrase,
        warehouse,
        database,
        schema,
        role,
    })
}
