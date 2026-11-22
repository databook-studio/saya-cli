// The old surface was this seam's only caller; X1 deleted it. The code here
// is X2/X3's to remove, kept compiled rather than deleted twice.
#[allow(dead_code)]
pub(crate) mod automatic;
// The old surface was this seam's only caller; X1 deleted it. The code here
// is X2/X3's to remove, kept compiled rather than deleted twice.
#[allow(dead_code)]
pub(crate) mod engine;
// The headless settle went with the old turn body: the plain REPL's turn is a
// chat session turn now, and the chat layer's compactor (the unit that owns
// between-turn compaction) is its caller. Kept annotated until that unit lands.
#[allow(dead_code)]
pub(crate) mod headless;
pub(crate) mod manual;
pub(crate) mod provider;
