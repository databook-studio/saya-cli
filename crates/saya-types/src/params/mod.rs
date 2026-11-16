//! Typed runtime parameters: declared specifications for saved
//! investigations and the validated values bound to them at run time.
//!
//! Values never persist: a [`ParamValue`] lives in the request that carries
//! it, and its [`Debug`] output prints the variant only, never the value.

mod error;
mod parse;
mod spec;
mod value;

pub use error::ParamError;
pub use spec::{
    BoundParam, MAX_PARAM_DESCRIPTION_BYTES, MAX_PARAM_NAME_CHARS, MAX_PARAMETERS, ParameterSpec,
    is_valid_param_name,
};
pub use value::{ParamType, ParamValue};

#[cfg(test)]
mod tests;
