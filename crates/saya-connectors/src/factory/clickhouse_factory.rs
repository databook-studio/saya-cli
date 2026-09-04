use saya_config::SecretResolver;
use saya_types::{ConnectionError, DatabaseProfile};

use crate::{ClickHouseConnector, ConnectorOptions};

pub(super) fn build(
    profile: &DatabaseProfile,
    resolver: &dyn SecretResolver,
    settings: ConnectorOptions,
) -> Result<Box<dyn crate::DatabaseConnector>, ConnectionError> {
    let DatabaseProfile::ClickHouse {
        host,
        port,
        database,
        user,
        password,
        secure,
    } = profile
    else {
        unreachable!()
    };
    let password = password
        .as_ref()
        .map(|reference| {
            resolver
                .resolve(reference)
                .map(|value| value.expose().to_owned())
                .map_err(|_| {
                    ConnectionError::invalid_configuration(
                        "ClickHouse authentication secret could not be resolved",
                    )
                })
        })
        .transpose()?;
    Ok(Box::new(ClickHouseConnector::new(
        host.clone(),
        *port,
        database.clone(),
        user.clone(),
        password,
        *secure,
        settings,
    )?))
}
