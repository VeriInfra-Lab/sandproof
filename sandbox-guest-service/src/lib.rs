#![forbid(unsafe_code)]
//! Guest service composition under development. These initialization routes
//! are not an installed envd replacement. Command/file registration, account
//! integration, listener admission and immutable-template integration remain
//! separate obligations; health does not certify authorization or readiness.

pub mod upload_body;

#[cfg(target_os = "linux")]
pub mod command;
#[cfg(target_os = "linux")]
mod execution;
#[cfg(target_os = "linux")]
pub mod file_io;
#[cfg(target_os = "linux")]
pub mod file_transfer;
#[cfg(target_os = "linux")]
pub mod files;
#[cfg(target_os = "linux")]
mod http;
#[cfg(target_os = "linux")]
mod metadata;
#[cfg(target_os = "linux")]
mod session;
#[cfg(target_os = "linux")]
pub use execution::{
    AccountPool, AccountSource, ContextError, ContextLimits, ExecutionSettings, SystemAccounts,
};
#[cfg(target_os = "linux")]
pub use http::initialization_routes;
#[cfg(target_os = "linux")]
pub use metadata::{MetadataLimits, MmdsClient};
#[cfg(target_os = "linux")]
pub use session::{
    AuthPermit, Defaults, InitLimits, Metadata, MetadataSource, ServiceError, Session,
};
