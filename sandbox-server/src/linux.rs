//! Linux service composition; modules own their respective obligations.

mod config;
pub(crate) mod host;
mod preflight;
mod reconcile;
pub(super) mod startup;

#[cfg(test)]
mod test_support;
