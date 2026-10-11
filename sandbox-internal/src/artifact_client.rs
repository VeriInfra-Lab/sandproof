//! Fixed protected artifact source, using the same pinned streaming transport.
use crate::cache::{ARTIFACT_PATH, ARTIFACT_REQUEST_BYTES, ArtifactRequest};
use crate::{ClientTls, InternalClient, Limits, RegisteredEndpoint, TransportError};
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Request, header};
use sandproof_controller_service::templates::TemplateArtifact;
use sandproof_protocol::internal::NodeIdentity;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Write, net::SocketAddr, sync::Arc, time::Duration};
use tokio::time::{Instant, timeout};

#[derive(Clone)]
pub struct ArtifactClient {
    transport: InternalClient,
    download_timeout: Duration,
    idle_timeout: Duration,
}
impl ArtifactClient {
    /// # Errors
    /// Rejects invalid fixed endpoints and zero/unrepresentable time budgets.
    pub fn new(
        tls: ClientTls,
        address: SocketAddr,
        server_name: String,
        node: NodeIdentity,
        limits: Limits,
        download_timeout: Duration,
        idle_timeout: Duration,
    ) -> Result<Self, TransportError> {
        if idle_timeout.is_zero()
            || idle_timeout > download_timeout
            || download_timeout > Duration::from_hours(24)
        {
            return Err(TransportError::Configuration);
        }
        Ok(Self {
            transport: InternalClient::new(
                tls,
                RegisteredEndpoint::new(address, server_name, node)?,
                limits,
            ),
            download_timeout,
            idle_timeout,
        })
    }
    /// Write only to the caller's already opened exact temporary inode. No
    /// descriptor path/redirect/archive is interpreted. The host cache caller
    /// owns admission, total temporary disk capacity, fsync and atomic publish.
    /// # Errors
    /// Wrong identity, framing, byte count, SHA-256, deadline or I/O leaves a
    /// partial file and UNKNOWN outcome; it never establishes availability.
    pub async fn download_to(
        &self,
        request: &ArtifactRequest,
        expected: &TemplateArtifact,
        output: &mut File,
    ) -> Result<(), TransportError> {
        if request.node != *self.transport.node_identity() || !expected.is_valid() {
            return Err(TransportError::Rejected);
        }
        let encoded = serde_json::to_vec(request).map_err(|_| TransportError::Rejected)?;
        if encoded.len() > ARTIFACT_REQUEST_BYTES {
            return Err(TransportError::Rejected);
        }
        let deadline = Instant::now()
            .checked_add(self.download_timeout)
            .ok_or(TransportError::Configuration)?;
        let request = Request::post(ARTIFACT_PATH)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT_ENCODING, "identity")
            .body(Body::from(encoded))
            .map_err(|_| TransportError::Configuration)?;
        let response = self
            .transport
            .stream_request(request, Arc::new(()), deadline)
            .await?;
        if response.status() != hyper::StatusCode::OK
            || response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(hyper::header::HeaderValue::as_bytes)
                != Some(b"application/octet-stream")
            || response.headers().contains_key(header::CONTENT_ENCODING)
            || response.headers().contains_key(header::TRANSFER_ENCODING)
            || response
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                != Some(expected.size_bytes)
        {
            return Err(TransportError::InvalidResponse);
        }
        let mut body = response.into_body();
        let mut bytes = 0_u64;
        let mut digest = Sha256::new();
        loop {
            let Some(frame) = timeout(self.idle_timeout, body.frame())
                .await
                .map_err(|_| TransportError::Deadline)?
            else {
                break;
            };
            let frame = frame.map_err(|_| TransportError::Communication)?;
            let data = frame
                .into_data()
                .map_err(|_| TransportError::InvalidResponse)?;
            bytes = bytes
                .checked_add(data.len() as u64)
                .filter(|value| *value <= expected.size_bytes)
                .ok_or(TransportError::InvalidResponse)?;
            for chunk in data.chunks(64 * 1024) {
                if Instant::now() >= deadline {
                    return Err(TransportError::Deadline);
                }
                output
                    .write_all(chunk)
                    .map_err(|_| TransportError::Communication)?;
                digest.update(chunk);
            }
        }
        if bytes != expected.size_bytes || format!("{:x}", digest.finalize()) != expected.sha256 {
            return Err(TransportError::InvalidResponse);
        }
        Ok(())
    }
}
