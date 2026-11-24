//! The built-in run plugins.
//!
//! A plugin contributes tool definitions, an executor, untrusted context, and
//! turn policy to a run through [`saya_agent::RunPlugin`]. The coding job's tool
//! surface already hands the database plugin to a [`saya_agent::PluginRegistry`]
//! and routes its names through it; the chat surface still assembles the same
//! tools itself, and its wiring through a registry comes later.

pub(crate) mod database;
