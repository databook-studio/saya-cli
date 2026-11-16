//! `--write .` states the whole repository, and an invalid write scope names
//! the offending path and the fix rather than a generic request error.
use saya_cli::{CodingError, CodingJobRequest};
use saya_types::JobId;

fn request(paths: &[&str]) -> Result<CodingJobRequest, CodingError> {
    CodingJobRequest::new(
        JobId::parse("write-scope").unwrap(),
        "edit the repository".into(),
        paths.iter().map(|path| (*path).to_string()).collect(),
    )
}

#[test]
fn a_dot_write_path_is_accepted_as_the_whole_repository() {
    let request = request(&["."]).expect("`.` names the whole repository");
    assert_eq!(request.job_id().as_str(), "write-scope");
}

#[test]
fn an_escaping_write_path_names_the_reason_and_the_fix() {
    let error = request(&["../outside"]).expect_err("an escaping path is refused");
    let CodingError::WritePaths { detail } = &error else {
        panic!("expected a write-path error, got {error:?}");
    };
    assert!(
        detail.contains("not a safe repository-relative path"),
        "the error names the reason: {detail}"
    );
    assert!(
        detail.contains("`."),
        "the error says how to state the whole repository: {detail}"
    );
}
