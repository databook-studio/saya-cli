use crate::{JobFile, SettingSource, provenance::Recorder};

pub(crate) fn merge(
    base: &mut JobFile,
    layer: &JobFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    macro_rules! apply_clone {
        ($field:ident) => {
            if layer.$field.is_some() {
                base.$field = layer.$field.clone();
                recorder.record(
                    concat!("job.", stringify!($field)),
                    source(stringify!($field)),
                );
            }
        };
    }
    apply_clone!(automatic_lsp_profile);
    apply_clone!(program_dir);
    apply_clone!(root_mode);
    if let Some(profiles) = layer.extensions.as_ref() {
        base.extensions = Some(profiles.clone());
        recorder.record("job.extensions", source("extensions"));
    }
}
