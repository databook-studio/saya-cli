use saya_config::SecretResolver;
use saya_types::{ConnectionError, DatabaseProfile};

use crate::{BigQueryConnector, ConnectorOptions};

pub(super) fn build(
    profile: &DatabaseProfile,
    resolver: &dyn SecretResolver,
    settings: ConnectorOptions,
) -> Result<Box<dyn crate::DatabaseConnector>, ConnectionError> {
    let DatabaseProfile::BigQuery {
        project,
        dataset,
        location,
        max_bytes_billed,
        service_account_key,
    } = profile
    else {
        unreachable!()
    };
    let key_json = resolver
        .resolve(service_account_key)
        .map(|value| value.expose().to_owned())
        .map_err(|_| {
            ConnectionError::invalid_configuration(
                "BigQuery service-account key could not be resolved",
            )
        })?;
    Ok(Box::new(BigQueryConnector::new(
        project.clone(),
        dataset.clone(),
        location.clone(),
        *max_bytes_billed,
        key_json,
        settings,
    )?))
}
