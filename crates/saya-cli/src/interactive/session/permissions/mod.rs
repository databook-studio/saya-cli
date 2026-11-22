pub(crate) mod activation;
pub(crate) mod allow_refusal;
pub(crate) mod deny;
pub(crate) mod grant_families;
pub(crate) mod grant_surface;
pub(crate) mod grants;
// The old surface was this seam's only caller; X1 deleted it. The code here
// is X2/X3's to remove, kept compiled rather than deleted twice.
#[allow(dead_code)]
pub(crate) mod mode;
pub(crate) mod trust;

#[cfg(test)]
mod deny_residual_tests;

#[cfg(test)]
mod deny_shell_tests;

#[cfg(test)]
mod deny_wrapper_tests;
