//! The two `[agents]` bounds: the defaults a job runs under, and the range
//! each one accepts.

use super::resolve;
use crate::{AgentsFile, ConfigError, ConfigFile, ResolvedAgents};

fn resolved(toml: &str) -> ResolvedAgents {
    let file = ConfigFile::from_toml(toml).expect("the fixture parses");
    resolve(&file.agents).expect("the fixture resolves")
}

fn refused(toml: &str) -> ConfigError {
    let file = ConfigFile::from_toml(toml).expect("the fixture parses");
    resolve(&file.agents).expect_err("the fixture is refused")
}

/// An empty table resolves exactly the defaults a job ran under before the
/// table existed: three children, four model requests, the two built-ins.
#[test]
fn an_empty_table_is_the_default_lane() {
    let agents = resolved("");
    assert_eq!(agents, ResolvedAgents::default());
    assert_eq!(agents.max_children, 3);
    assert_eq!(agents.max_concurrent_model_requests, 4);
    assert_eq!(agents.types.known_names(), "explore, general");
    assert_eq!(resolve(&AgentsFile::default()).unwrap(), agents);
}

/// Both bounds are stated in the same table as the types, and each keeps its
/// own range: a child bound of zero is refused, and so is one over three.
#[test]
fn each_bound_keeps_its_own_range() {
    let agents = resolved("[agents]\nmax_children = 1\nmax_concurrent_model_requests = 2\n");
    assert_eq!(agents.max_children, 1);
    assert_eq!(agents.max_concurrent_model_requests, 2);

    for (setting, value, min, max) in [
        ("max_children", 0, 1, 3),
        ("max_children", 4, 1, 3),
        ("max_concurrent_model_requests", 1, 2, 4),
        ("max_concurrent_model_requests", 5, 2, 4),
    ] {
        let error = refused(&format!("[agents]\n{setting} = {value}\n"));
        assert!(
            matches!(
                &error,
                ConfigError::SettingOutOfRange {
                    field,
                    value: actual,
                    min: actual_min,
                    max: actual_max,
                } if *field == format!("agents.{setting}")
                    && *actual == value
                    && *actual_min == min
                    && *actual_max == max
            ),
            "expected the range refusal for agents.{setting} = {value}: {error}"
        );
    }
}
