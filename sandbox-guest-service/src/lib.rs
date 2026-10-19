#![forbid(unsafe_code)]
//! Guest service implementation under development. `GuestHttp` combines existing
//! initialization, command and file handlers with one shared session and an
//! explicitly configured bounded listener. The Linux daemon entry consumes one
//! explicit configuration and separates known-new VM initialization from normal
//! startup. Immutable-template integration remains an independent obligation;
//! this crate does not install a replacement. Health does not certify readiness.

#[cfg(target_os = "linux")]
mod admission_response;
mod blocking_executor;
pub mod build_info;
#[cfg(any(target_os = "linux", test))]
mod command_clock;
pub mod command_keepalive;
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
#[cfg(unix)]
pub mod startup;
pub mod task_accounting;
pub mod upload_body;
pub mod upload_response;

#[cfg(any(target_os = "linux", test))]
mod basic_auth;
#[cfg(target_os = "linux")]
pub mod command;
#[cfg(target_os = "linux")]
pub mod command_http;
#[cfg(target_os = "linux")]
pub mod daemon;
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
