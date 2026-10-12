//! Read-only registered artifacts, selected by exact TLS peer and authority.
use super::stream::ArtifactBody;
use axum::body::Body;
use http_body_util::{BodyExt, Limited};
use hyper::{Method, Request, Response, StatusCode, body::Incoming, header};
use sandproof_center_service::{business::SharedBusinessAuthority, deployment::ArtifactPeer};
use sandproof_controller_service::template_registry::CandidateStore;
use sandproof_internal::{
    CertificatePin, ProtectedHttp,
    cache::{ARTIFACT_PATH, ARTIFACT_REQUEST_BYTES, ArtifactPart, ArtifactRequest},
    check_protected_directory,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::{
    runtime::Handle,
    sync::{Semaphore, watch},
    time::Instant,
};

pub(super) struct ArtifactHttp {
    authority: SharedBusinessAuthority,
    store: Arc<CandidateStore>,
    peers: Vec<ArtifactPeer>,
    owner: u32,
    deadline: Duration,
    reads: Arc<Semaphore>,
    observations: Arc<Semaphore>,
    runtime: Handle,
}
impl ArtifactHttp {
    pub(super) fn new(
        authority: SharedBusinessAuthority,
        store: Arc<CandidateStore>,
        peers: Vec<ArtifactPeer>,
        owner: u32,
        deadline: Duration,
        max_reads: usize,
        runtime: Handle,
    ) -> Self {
        Self {
            authority,
            store,
            peers,
            owner,
            deadline,
            reads: Arc::new(Semaphore::new(max_reads)),
            observations: Arc::new(Semaphore::new(1)),
            runtime,
        }
    }
    async fn readiness(
        &self,
        peer: CertificatePin,
        request: Request<Incoming>,
    ) -> Result<Response<Body>, StatusCode> {
        use sandproof_internal::cache::{READINESS_RESPONSE_BYTES, ReadinessRequest};
        if request.method() != Method::POST
            || request.uri().query().is_some()
            || request.headers().contains_key(header::CONTENT_ENCODING)
            || request
                .headers()
                .get(header::CONTENT_TYPE)
                .map(hyper::header::HeaderValue::as_bytes)
                != Some(b"application/json")
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let known = self
            .peers
            .iter()
            .find(|known| known.pins.contains(&peer))
            .ok_or(StatusCode::FORBIDDEN)?;
        let bytes = Limited::new(request.into_body(), ARTIFACT_REQUEST_BYTES)
            .collect()
            .await
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?
            .to_bytes();
        let request: ReadinessRequest =
            serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
        if request.node != known.node {
            return Err(StatusCode::FORBIDDEN);
        }
        let permit = Arc::clone(&self.observations)
            .try_acquire_owned()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let authority = self.authority.clone();
        let bytes = self
            .runtime
            .spawn_blocking(move || {
                let _permit = permit;
                let original = authority
                    .snapshot()
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                let node = original
                    .center
                    .nodes()
                    .iter()
                    .find(|node| node.identity == request.node)
                    .ok_or(StatusCode::FORBIDDEN)?;
                let mut catalog = Vec::new();
                for (revision, binding) in original
                    .metadata
                    .static_artifacts()
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
                {
                    if original.metadata.artifact_role(&revision) == node.role {
                        catalog.push(sandproof_center_core::AvailableTemplate {
                            revision,
                            digest: binding
                                .canonical_digest()
                                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
                        });
                    }
                }
                // One confirmed original snapshot, no disk scans or state setters.
                // No paths, API keys, tokens or other tenants' lifecycle records.
                let encoded = serde_json::to_vec(&serde_json::json!({
                    "schema_version": 1, "node": node, "catalog": catalog,
                }))
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                if encoded.len() > READINESS_RESPONSE_BYTES {
                    return Err(StatusCode::SERVICE_UNAVAILABLE);
                }
                Ok::<_, StatusCode>(encoded)
            })
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, bytes.len())
            .body(Body::from(bytes))
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
    async fn artifact(
        &self,
        peer: CertificatePin,
        request: Request<Incoming>,
        streaming: watch::Sender<bool>,
    ) -> Result<Response<Body>, StatusCode> {
        if request.method() != Method::POST
            || request.uri().path() != ARTIFACT_PATH
            || request.uri().query().is_some()
        {
            return Err(StatusCode::NOT_FOUND);
        }
        if request.headers().contains_key(header::CONTENT_ENCODING)
            || request
                .headers()
                .get(header::CONTENT_TYPE)
                .map(hyper::header::HeaderValue::as_bytes)
                != Some(b"application/json")
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let exact_peer = self
            .peers
            .iter()
            .find(|known| known.pins.contains(&peer))
            .ok_or(StatusCode::FORBIDDEN)?;
        let bytes = Limited::new(request.into_body(), ARTIFACT_REQUEST_BYTES)
            .collect()
            .await
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?
            .to_bytes();
        let request: ArtifactRequest =
            serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
        if request.node != exact_peer.node {
            return Err(StatusCode::FORBIDDEN);
        }
        let until = Instant::now()
            .checked_add(self.deadline)
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let permit = Arc::clone(&self.reads)
            .try_acquire_owned()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let authority = self.authority.clone();
        let store = Arc::clone(&self.store);
        let owner = self.owner;
        // The worker, not its canceled HTTP waiter, retains this finite slot.
        let (file, size) = self
            .runtime
            .spawn_blocking(move || {
                let _permit = permit;
                let original = authority
                    .snapshot()
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                if !original
                    .center
                    .nodes()
                    .iter()
                    .any(|node| node.identity == request.node)
                {
                    return Err(StatusCode::FORBIDDEN);
                }
                let binding = original
                    .metadata
                    .artifact_binding(&request.revision, &request.digest)
                    .map_err(|_| StatusCode::NOT_FOUND)?;
                let artifact = match request.part {
                    ArtifactPart::Kernel => &binding.kernel,
                    ArtifactPart::Rootfs => &binding.rootfs,
                };
                let parent = artifact
                    .path
                    .parent()
                    .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
                check_protected_directory(parent, owner)
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                let file = store
                    .open_artifact(artifact)
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                Ok::<_, StatusCode>((file, artifact.size_bytes))
            })
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
        let body = Body::new(ArtifactBody::new(file, size, until));
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, size)
            .body(body)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let _ = streaming.send(true);
        Ok(response)
    }
}
impl ProtectedHttp for ArtifactHttp {
    async fn respond(
        &self,
        peer: CertificatePin,
        request: Request<Incoming>,
        streaming: watch::Sender<bool>,
    ) -> Result<Response<Body>, Infallible> {
        let result = if request.uri().path() == sandproof_internal::cache::READINESS_PATH {
            self.readiness(peer, request).await
        } else {
            self.artifact(peer, request, streaming).await
        };
        Ok(match result {
            Ok(response) => response,
            Err(status) => sandproof_data_transport::response(status),
        })
    }
}
