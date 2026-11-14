//! The root mode a coding launch works over, resolved from its flag, its
//! configured default, and — for a lifecycle verb — the job's own record.

use super::CodingCommand;
use saya_config::JobRootMode;
use std::path::Path;

/// The one root a coding application opens: the folder itself, or a managed
/// checkout copied from a Git repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootMode {
    InPlace,
    Managed,
}

impl CodingCommand {
    /// The root mode opening the application for this command: a launch states
    /// it, a lifecycle verb follows the job's own record, and everything else
    /// follows the configured default.
    pub fn root_mode_for(&self, state_root: &Path, configured: JobRootMode) -> RootMode {
        if let Self::Start {
            in_place, managed, ..
        } = self
        {
            return match (in_place, managed) {
                (_, true) => RootMode::Managed,
                (true, false) => RootMode::InPlace,
                (false, false) => configured.into(),
            };
        }
        let Some(id) = self.job_id() else {
            return configured.into();
        };
        let records = state_root.join(id);
        if records.join("inplace-root.json").is_file() {
            RootMode::InPlace
        } else if records.is_dir() {
            RootMode::Managed
        } else {
            configured.into()
        }
    }
}

impl From<JobRootMode> for RootMode {
    fn from(mode: JobRootMode) -> Self {
        match mode {
            JobRootMode::InPlace => Self::InPlace,
            JobRootMode::Managed => Self::Managed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn start(in_place: bool, managed: bool) -> CodingCommand {
        CodingCommand::Start {
            id: "job-a".into(),
            task: "edit the folder".into(),
            write_paths: vec![".".into()],
            database_profiles: Vec::new(),
            tools: Vec::new(),
            plan: false,
            approve: true,
            in_place,
            managed,
        }
    }

    fn state_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "saya-coding-root-mode-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// A start states its flag first, then the configured default; a listing
    /// or readiness check has no launch and follows the config.
    #[test]
    fn flag_beats_config() {
        let root = state_root("flag");
        assert_eq!(
            start(false, false).root_mode_for(&root, JobRootMode::InPlace),
            RootMode::InPlace,
            "no flag follows the configured in_place"
        );
        assert_eq!(
            start(false, false).root_mode_for(&root, JobRootMode::Managed),
            RootMode::Managed,
            "no flag follows the configured managed"
        );
        assert_eq!(
            start(true, false).root_mode_for(&root, JobRootMode::Managed),
            RootMode::InPlace,
            "--in-place forces the folder over a managed config"
        );
        assert_eq!(
            start(false, true).root_mode_for(&root, JobRootMode::InPlace),
            RootMode::Managed,
            "--managed forces a checkout over an in_place config"
        );
        assert_eq!(
            CodingCommand::List { after: None }.root_mode_for(&root, JobRootMode::Managed),
            RootMode::Managed
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// A lifecycle verb follows the job's private record: the in-place record
    /// means in place, a job directory without one means managed, and a job
    /// that does not exist follows the config. The launch flags are ignored.
    #[test]
    fn a_lifecycle_verb_follows_the_record() {
        let root = state_root("record");
        let in_place_id = root.join("job-in-place");
        fs::create_dir_all(&in_place_id).unwrap();
        fs::write(in_place_id.join("inplace-root.json"), b"{}").unwrap();
        let managed_id = root.join("job-managed");
        fs::create_dir_all(&managed_id).unwrap();
        fs::write(managed_id.join("root-checkout.json"), b"{}").unwrap();

        let show_in_place = CodingCommand::Show {
            id: "job-in-place".into(),
        };
        let show_managed = CodingCommand::Show {
            id: "job-managed".into(),
        };
        assert_eq!(
            show_in_place.root_mode_for(&root, JobRootMode::Managed),
            RootMode::InPlace,
            "the record wins over a managed config"
        );
        assert_eq!(
            show_managed.root_mode_for(&root, JobRootMode::InPlace),
            RootMode::Managed,
            "a job directory without the record is a managed job"
        );
        assert_eq!(
            CodingCommand::Show {
                id: "job-missing".into()
            }
            .root_mode_for(&root, JobRootMode::Managed),
            RootMode::Managed,
            "an unknown job follows the config"
        );

        let follow_up = CodingCommand::FollowUp {
            id: "job-in-place".into(),
            agent: "agent-a".into(),
            expected_completion_sequence: 1,
            instruction: "repair".into(),
            approve: true,
            in_place: false,
            managed: true,
        };
        assert_eq!(
            follow_up.root_mode_for(&root, JobRootMode::Managed),
            RootMode::InPlace,
            "a follow-up ignores both launch flags"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
