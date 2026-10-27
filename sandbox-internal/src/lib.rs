#![forbid(unsafe_code)]

//! Bounded, mutually authenticated center/host transport.
//! Shared authenticated transport for identity probes and high-level lifecycle
//! intents. The center is the sole public product entrance.
//! TLS material, registered endpoint and peer pins come from trusted assembly,
//! never from a user/template or an HTTP header. Entrypoints explicitly choose
//! paths/owners/bounds; a shared protected-file primitive avoids two loaders.

mod artifact_client;
pub mod cache;
mod client;
pub mod diagnostic;
pub mod lifecycle;
#[cfg(unix)]
mod protected_files;
mod protected_server;
mod server;
pub mod status;
pub mod template;
mod tls;

pub use artifact_client::ArtifactClient;
pub use cache::CacheReceiver;
pub use client::{InternalClient, RegisteredEndpoint};
#[cfg(unix)]
pub use protected_files::{ProtectedFileError, check_protected_directory, read_protected};
pub use protected_server::{ProtectedHttp, ProtectedServer};
pub use server::{DataReceiver, InternalServer, LifecycleReceiver, ReceiveError, RejectLifecycle};
pub use template::ValidationReceiver;
pub use tls::{CertificatePin, ClientTls, ServerTls, TlsIdentity};

use std::{error::Error, fmt, time::Duration};

// Both HTTP parsers use the same explicit budget; body limits are separate.
pub(crate) const HTTP_MAX_HEADERS: usize = 16;
pub(crate) const HTTP_HEADER_BUFFER_BYTES: usize = 8192;

/// Explicit deployment budgets; no semaphore wait queue is created.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub(crate) max_connections: usize,
    pub(crate) handshake_timeout: Duration,
    pub(crate) request_timeout: Duration,
}

impl Limits {
    #[must_use]
    pub const fn max_connections(&self) -> usize {
        self.max_connections
    }
    /// # Errors
    /// Rejects zero/oversized budgets and handshake deadlines beyond the call.
    pub fn new(
        max_connections: usize,
        handshake_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, TransportError> {
        if !(1..=8192).contains(&max_connections)
            || handshake_timeout.is_zero()
            || handshake_timeout > request_timeout
            || request_timeout > Duration::from_hours(24)
        {
            return Err(TransportError::Configuration);
        }
        Ok(Self {
            max_connections,
            handshake_timeout,
            request_timeout,
        })
    }
}

/// A communication failure is never definitive lifecycle failure/no effects.
/// Callers of future effectful endpoints must retain the original operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    Configuration,
    Overloaded,
    Deadline,
    Authentication,
    Communication,
    InvalidResponse,
    Rejected,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "invalid internal transport configuration",
            Self::Overloaded => "internal transport capacity exhausted",
            Self::Deadline => "internal call deadline exceeded",
            Self::Authentication => "internal peer authentication failed",
            Self::Communication => "internal communication unavailable",
            Self::InvalidResponse => "invalid internal response",
            Self::Rejected => "internal request rejected",
        })
    }
}

impl Error for TransportError {}

// Private verification candidate only.
pub use server::{
    ProofHttpJsonResponseBuilder, ProofJsonResponseBuilder, proof_json_response_result,
};
