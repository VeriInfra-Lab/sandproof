//! MIME lookup/sniffing and download disposition for the fixed Go endpoint.
//! No global table or HTTP listener. The daemon must supply its actual bounded
//! MIME-file snapshot; builtins alone are not a claim about the deployed table.
mod builtins;
mod catalog;
mod format;
mod sniff;
#[cfg(target_os = "linux")]
mod system;
pub use catalog::{Catalog, CatalogLimits};
pub use format::{basename, disposition, extension};
pub use sniff::{SNIFF_BYTES, detect};
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    InputLimit,
    RetainedLimit,
    UnsupportedText,
    Io,
}
