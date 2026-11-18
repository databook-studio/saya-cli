//! The built-in run plugins.
//!
//! A plugin contributes tool definitions, an executor, untrusted context, and
//! turn policy to a run through [`saya_agent::RunPlugin`]. Nothing here is wired
//! into a run path yet: the core still assembles its own tools, and the surface
//! that hands these plugins to a [`saya_agent::PluginRegistry`] comes later.

pub(crate) mod database;
