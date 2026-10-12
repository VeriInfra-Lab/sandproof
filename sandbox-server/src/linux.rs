//! Linux service composition; modules own their respective obligations.

mod config;
pub(super) mod host_entry;
mod preflight;
mod reconcile;

#[cfg(test)]
mod test_support;
