//! Tests for the saved-investigation contract.

use super::*;
use crate::params::{MAX_PARAMETERS, ParamError, ParamType, ParameterSpec};

const CREATED: i64 = 1_700_000_000_000;

const VERSION_TWO_JSON: &[u8] = concat!(
    r#"{"format":"saya.investigation","version":2,"id":"rev-12345678","revision":1,"#,
    r#""name":"n","sql":"select 1","dialect":"postgresql","connection":"c","#,
    r#""objects":[],"schema_fingerprint":null,"created_unix_ms":1,"updated_unix_ms":1}"#,
)
.as_bytes();

const WRONG_FORMAT_JSON: &[u8] = br#"{"format":"saya.query","version":1}"#;

const MISSING_VERSION_JSON: &[u8] = br#"{"format":"saya.investigation","id":"rev-12345678"}"#;

const INVALID_ID_JSON: &[u8] = concat!(
    r#"{"format":"saya.investigation","version":1,"id":"Bad","revision":1,"name":"n","#,
    r#""sql":"select 1","dialect":"postgresql","connection":"c","objects":[],"#,
    r#""schema_fingerprint":null,"created_unix_ms":1,"updated_unix_ms":1}"#,
)
.as_bytes();

fn valid_definition() -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_string(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: InvestigationId::derive("revenue", "select 1", CREATED),
        revision: 1,
        name: "Revenue".to_string(),
        description: None,
        sql: "select 1".to_string(),
        parameters: Vec::new(),
        dialect: SqlDialect::Postgres,
        connection: "prod".to_string(),
        objects: vec!["public.orders".to_string()],
        schema_fingerprint: None,
        created_unix_ms: CREATED,
        updated_unix_ms: CREATED,
    }
}

fn definition_json(definition: &InvestigationDefinitionV1) -> String {
    serde_json::to_string_pretty(definition).expect("a valid definition serializes")
}

fn parameter_spec(name: &str, param_type: ParamType) -> ParameterSpec {
    ParameterSpec {
        name: name.to_owned(),
        param_type,
        required: false,
        description: None,
    }
}

fn definition_with_parameters() -> InvestigationDefinitionV1 {
    let mut definition = valid_definition();
    definition.sql = "select * from orders where region = :region".to_string();
    definition.parameters = vec![parameter_spec("region", ParamType::String)];
    definition
}

#[path = "tests/collection_validation.rs"]
mod collection_validation;
#[path = "tests/identity.rs"]
mod identity;
#[path = "tests/text_validation.rs"]
mod text_validation;
#[path = "tests/wire_format.rs"]
mod wire_format;
