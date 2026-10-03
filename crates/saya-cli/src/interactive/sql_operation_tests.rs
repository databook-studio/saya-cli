use super::{SqlOperationError, SqlOperationPhase};
use saya_types::ConnectionError;
use std::error::Error;

#[test]
fn sql_errors_retain_their_source_and_operation_phase() {
    let errors = [
        (
            SqlOperationError::Build(ConnectionError::invalid_configuration("build")),
            SqlOperationPhase::Build,
        ),
        (
            SqlOperationError::Connect(ConnectionError::invalid_configuration("connect")),
            SqlOperationPhase::Connect,
        ),
        (
            SqlOperationError::Execute(ConnectionError::invalid_configuration("execute")),
            SqlOperationPhase::Execute,
        ),
    ];

    for (error, phase) in errors {
        assert_eq!(error.phase(), Some(phase));
        assert!(error.source().is_some());
    }
}
