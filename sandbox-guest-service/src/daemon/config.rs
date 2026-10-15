//! One explicit configuration for the original guest components. Decoding is
//! not validation or permission to serve. There are no environment fallbacks.
use crate::{
    InitLimits, MetadataLimits, command, command_http, command_requests, file_requests,
    file_rpc_requests, file_rpc_response, file_transfer, files, listener, startup, upload_response,
};
use sandproof_guest_runtime::{
    ProcessLimits,
    accounts::AccountLimits,
    file_download::local_zone,
    file_mime::CatalogLimits,
    filesystem::{FileLimits, transfer::TransferLimits},
};
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, time::Duration};

/// Ceiling for the trusted configuration document, not a request/body limit or
/// total daemon memory claim. The same ceiling applies before JSON allocation.
pub const CONFIG_BYTES: usize = 1_048_576;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub version: u32,
    pub listen: SocketAddr,
    pub runtime: Runtime,
    pub paths: Paths,
    pub startup: startup::Limits,
    pub metadata: MetadataLimits,
    pub initialization: InitLimits,
    pub transport: listener::Limits,
    pub accounts: Accounts,
    pub commands: Commands,
    pub files: Files,
    pub snapshots: Snapshots,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    pub worker_threads: usize,
    pub blocking_threads: usize,
    pub thread_stack_bytes: usize,
    pub shutdown_timeout: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    pub owner_uid: u32,
    pub pid_directory: PathBuf,
    pub worker: PathBuf,
    pub gate_directory: PathBuf,
    pub max_pid: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accounts {
    pub source: AccountLimits,
    pub in_flight: usize,
    pub timeout: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commands {
    pub context: crate::ExecutionSettings,
    pub process: ProcessLimits,
    pub owners: command::OwnerLimits,
    pub admission: command::AdmissionLimits,
    pub registry: command::RegistryLimits,
    pub ordinary: command_requests::Limits,
    pub control: command_requests::Limits,
    pub response: command_http::ResponseLimits,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Files {
    pub context: files::FileSettings,
    pub requests: file_requests::Limits,
    pub rpc_requests: file_rpc_requests::Limits,
    pub operations: Operations,
    pub transfers: Transfers,
    pub upload: file_transfer::UploadLimits,
    pub download: file_transfer::DownloadLimits,
    pub upload_response: upload_response::Limits,
    pub rpc_response: file_rpc_response::Limits,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operations {
    pub in_flight: usize,
    pub limits: FileLimits,
    pub timeout: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfers {
    pub in_flight: usize,
    pub limits: TransferLimits,
    pub timeout: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshots {
    pub mime: CatalogLimits,
    pub timezone: PathBuf,
    pub timezone_limits: local_zone::LoadLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Size,
    Json,
    Version,
}

impl Configuration {
    /// Decode a bounded complete document. All nested component fields are
    /// explicit; unknown, duplicate, missing or incorrectly typed fields fail.
    /// Durations use Serde's explicit {secs,nanos} representation.
    /// # Errors
    /// Oversized input, invalid JSON/schema or an unsupported schema version.
    /// Diagnostics deliberately retain no configuration text or filesystem path.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() > CONFIG_BYTES {
            return Err(DecodeError::Size);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| DecodeError::Json)?;
        if value.version != 1 {
            return Err(DecodeError::Version);
        }
        Ok(value)
    }
}
