//! Formal configuration and startup assembly under development. This module
//! does not install or replace a template. Only the original bounded handlers
//! may serve commands/files; startup must not introduce a parallel test route.
mod assembly;
pub mod config;
pub mod entry;
mod start;
pub use assembly::Prepared;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    Files,
    Snapshots,
    Startup(crate::startup::Error),
    DeadlineExceeded,
    Cancelled,
    WorkerUnavailable,
    Listener,
}
