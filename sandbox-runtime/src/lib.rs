#![forbid(unsafe_code)]

//! The I/O boundary for a small, single-host sandbox runtime.
//!
//! This crate is intentionally separate from the already-proved pure
//! controller. Nothing here is claimed to be covered by those proofs yet.

pub mod firecracker;
pub mod image;
pub mod import;
pub mod protocol;
pub mod result_store;
