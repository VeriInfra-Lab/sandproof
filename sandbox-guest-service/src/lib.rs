#![forbid(unsafe_code)]
//! Guest service composition under development. `GuestHttp` combines existing
//! initialization, command and file handlers with one shared session and an
//! explicitly configured bounded listener. It does not provision PID state or
//! install an envd replacement; trusted configuration and immutable-template
//! integration remain separate obligations. Health does not certify readiness.

pub mod command_requests;
mod connect_timeout;
pub mod file_authorization;
pub mod file_download;
pub mod file_encoding;
pub mod file_query;
pub mod file_requests;
pub mod file_rpc_requests;
pub mod file_rpc_response;
pub mod file_upload;
mod finite_body;
mod go_json;
mod go_quote;
pub mod listener;
mod owned_body;
mod request_gzip;
pub mod upload_body;
pub mod upload_response;

#[cfg(any(target_os = "linux", test))]
mod basic_auth;
#[cfg(target_os = "linux")]
pub mod command;
#[cfg(target_os = "linux")]
pub mod command_http;
#[cfg(target_os = "linux")]
mod execution;
#[cfg(target_os = "linux")]
pub mod file_http;
#[cfg(target_os = "linux")]
pub mod file_io;
#[cfg(target_os = "linux")]
pub mod file_rpc_http;
#[cfg(target_os = "linux")]
pub mod file_transfer;
#[cfg(target_os = "linux")]
pub mod files;
#[cfg(target_os = "linux")]
pub mod guest_http;
mod http;
#[cfg(target_os = "linux")]
mod metadata;
mod session;
#[cfg(target_os = "linux")]
pub use execution::{
    AccountPool, AccountSource, ContextError, ContextLimits, ExecutionSettings, SystemAccounts,
};
pub use http::initialization_routes;
#[cfg(target_os = "linux")]
pub use metadata::{MetadataLimits, MmdsClient};
pub use session::{
    AuthPermit, Defaults, InitLimits, Metadata, MetadataSource, ServiceError, Session,
};
