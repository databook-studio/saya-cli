/// Logical purpose of a request that can issue one or more physical sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EffectCallKind {
    Answer,
    Continuation,
    Salvage,
}

/// Identity that links one physical send to its logical receive and purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysicalAttempt {
    parent: super::coding_admission::EffectPermit,
    kind: EffectCallKind,
    ordinal: u8,
}

impl PhysicalAttempt {
    pub const fn new(
        parent: super::coding_admission::EffectPermit,
        kind: EffectCallKind,
        ordinal: u8,
    ) -> Self {
        Self {
            parent,
            kind,
            ordinal,
        }
    }

    pub const fn parent(self) -> super::coding_admission::EffectPermit {
        self.parent
    }

    pub const fn kind(self) -> EffectCallKind {
        self.kind
    }

    pub const fn ordinal(self) -> u8 {
        self.ordinal
    }
}
