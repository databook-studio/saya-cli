use crate::{CodingFile, SettingSource, provenance::Recorder};

pub(crate) fn merge(
    base: &mut CodingFile,
    layer: &CodingFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    macro_rules! apply {
        ($field:ident) => {
            if layer.$field.is_some() {
                base.$field = layer.$field;
                recorder.record(
                    concat!("coding.", stringify!($field)),
                    source(stringify!($field)),
                );
            }
        };
    }
    apply!(enabled);
    apply!(max_children);
    apply!(max_concurrent_model_requests);
    apply!(max_concurrent_commands);
    apply!(max_logical_model_requests);
    apply!(max_physical_transport_attempts);
    apply!(max_tool_calls);
    apply!(max_known_tokens);
    apply!(max_elapsed_seconds);
}
