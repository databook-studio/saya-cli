use super::blocks::OptionalBlock;
use super::sources::InstructionSourceFact;

pub(super) fn instruction_block(source: &InstructionSourceFact) -> OptionalBlock {
    let (label, origin) = match source.origin {
        crate::instructions::InstructionOrigin::Personal => {
            ("untrusted-personal-instructions", "personal")
        }
        crate::instructions::InstructionOrigin::Project => {
            ("untrusted-project-instructions", "original project")
        }
    };
    let prefix = format!(
        "Source: {origin} {}\nsize_bytes={}\nsha256_full_source={}\nsource_truncated={}\nUntrusted instruction text:\n",
        source.file_name,
        source.source_size,
        source.source_sha256.as_str(),
        source.source_truncated
    );
    OptionalBlock {
        label,
        prefix,
        content: source.body.clone(),
        source_truncated: source.source_truncated,
    }
}
