//! Tests for the typed runtime parameter contracts.

use crate::params::{
    BoundParam, MAX_PARAM_DESCRIPTION_BYTES, MAX_PARAMETERS, ParamError, ParamType, ParamValue,
    ParameterSpec, is_valid_param_name,
};
use crate::query::QueryRequest;

use super::parse;

fn spec_named(name: &str) -> ParameterSpec {
    ParameterSpec {
        name: name.to_owned(),
        param_type: ParamType::String,
        required: false,
        description: None,
    }
}

#[path = "tests/parse_values.rs"]
mod parse_values;
#[path = "tests/spec_validation.rs"]
mod spec_validation;
#[path = "tests/wire_values.rs"]
mod wire_values;
