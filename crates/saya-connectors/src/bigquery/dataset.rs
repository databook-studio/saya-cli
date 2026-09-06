//! Naming rules for the configured dataset: what is accepted, and which
//! project owns it.

/// Accepts the two shapes the connector documents: a bare dataset id, and a
/// dataset qualified by the project that owns it.
///
/// The value is interpolated into the `INFORMATION_SCHEMA` statement, so it is
/// checked against BigQuery's own naming rules — dataset ids are letters,
/// digits and underscores; project ids add the hyphen — rather than escaped.
/// Anything that could close the quoted identifier or begin a second statement
/// fails this check and is refused when the connector is built.
pub(crate) fn is_valid(dataset: &str) -> bool {
    match dataset.split_once('.') {
        Some((project, name)) => is_project_id(project) && is_dataset_id(name),
        None => is_dataset_id(dataset),
    }
}

/// Returns the project that owns the dataset and the bare dataset id. A public
/// dataset lives outside the project that pays for the query, so its
/// `INFORMATION_SCHEMA` must be read from the owning project; an unqualified
/// dataset belongs to the connector's own project.
pub(crate) fn split<'a>(dataset: &'a str, default_project: &'a str) -> (&'a str, &'a str) {
    match dataset.split_once('.') {
        Some((project, name)) => (project, name),
        None => (default_project, dataset),
    }
}

fn is_dataset_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_project_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_dataset_belongs_to_the_connectors_own_project() {
        assert_eq!(split("sales", "my-proj"), ("my-proj", "sales"));
    }

    #[test]
    fn qualified_dataset_names_the_project_that_owns_it() {
        // A public dataset lives outside the project that pays for the query,
        // so INFORMATION_SCHEMA must be read from the owning project or the
        // lookup resolves to a dataset that does not exist there.
        assert_eq!(
            split("bigquery-public-data.usa_names", "my-proj"),
            ("bigquery-public-data", "usa_names")
        );
    }

    #[test]
    fn accepts_the_two_documented_shapes() {
        assert!(is_valid("usa_names"));
        assert!(is_valid("bigquery-public-data.usa_names"));
        assert!(is_valid("_underscore_start"));
    }

    #[test]
    fn rejects_anything_that_could_escape_a_quoted_identifier() {
        // The dataset is interpolated into the INFORMATION_SCHEMA statement, so
        // a value that can close the backtick or start a new statement is
        // refused before it is ever formatted into SQL.
        for bad in [
            "",
            "has space",
            "back`tick",
            "semi;colon",
            "quote'mark",
            "a.b.c",
            "trailing.",
            ".leading",
            "dash-in-dataset",
        ] {
            assert!(!is_valid(bad), "accepted unsafe dataset: {bad:?}");
        }
    }
}
