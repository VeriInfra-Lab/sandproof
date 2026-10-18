#![forbid(unsafe_code)]
//! Pure global intent decisions. No Redis, HTTP, `SQLite`, clock or VM effects.
//! Time and trusted static registration are explicit inputs. This is NOT a
//! copy of the local VM lifecycle reducer. No public service is enabled here.
mod cache;
mod decision;
mod model;
mod validation;
mod window;
pub use decision::ResponseDisposition;
pub use model::*;
/// Pure syntax/frontier check used by recovery; it grants no authority.
pub use validation::number as parse_allocated_number;
#[cfg(test)]
mod tests;
