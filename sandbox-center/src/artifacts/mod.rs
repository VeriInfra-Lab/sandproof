//! Protected artifact production assembly; no local VM or second registry.
mod candidates;
mod http;
mod stream;
#[cfg(test)]
mod tests;

pub use candidates::ProtectedCandidates;
use sandproof_center_service::{
    RegisteredHosts, business::SharedBusinessAuthority, deployment::ArtifactAssembly,
};
use sandproof_controller_service::template_registry::CandidateStore;
use sandproof_internal::ProtectedServer;
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    runtime::Handle,
    sync::{oneshot, watch},
};

pub struct PreparedArtifacts {
    pub candidates: ProtectedCandidates,
    pub service: ArtifactService,
}
pub struct ArtifactService {
    bind: std::net::SocketAddr,
    server: ProtectedServer<http::ArtifactHttp>,
}
pub struct BoundArtifactService {
    listener: TcpListener,
    server: ProtectedServer<http::ArtifactHttp>,
}

pub fn assemble(
    authority: SharedBusinessAuthority,
    hosts: RegisteredHosts,
    inputs: ArtifactAssembly,
    runtime: Handle,
) -> Result<PreparedArtifacts, &'static str> {
    let original = authority
        .snapshot()
        .map_err(|_| "artifact authority unavailable")?;
    if original.metadata.registry().config().root != inputs.registry.root {
        return Err("original artifact root does not match protected configuration");
    }
    let store = Arc::new(
        CandidateStore::new(inputs.registry, inputs.owner_uid)
            .map_err(|_| "protected candidate store rejected")?,
    );
    let candidates = ProtectedCandidates::new(authority.clone(), hosts, Arc::clone(&store));
    let handler = http::ArtifactHttp::new(
        authority,
        store,
        inputs.peers,
        inputs.owner_uid,
        inputs.stream_timeout,
        inputs.limits.max_connections(),
        runtime,
    );
    let server = ProtectedServer::new(inputs.tls, inputs.limits, handler);
    Ok(PreparedArtifacts {
        candidates,
        service: ArtifactService {
            bind: inputs.bind,
            server,
        },
    })
}
impl ArtifactService {
    pub async fn listen(self) -> Result<BoundArtifactService, &'static str> {
        let listener = TcpListener::bind(self.bind)
            .await
            .map_err(|_| "protected artifact listener unavailable")?;
        Ok(BoundArtifactService {
            listener,
            server: self.server,
        })
    }
}
impl BoundArtifactService {
    pub async fn serve(self, mut shutdown: watch::Receiver<bool>) -> Result<(), &'static str> {
        let (stop, signal) = oneshot::channel();
        let serving = self.server.serve(self.listener, signal);
        tokio::pin!(serving);
        tokio::select! {
            result = &mut serving => result.map_err(|_| "protected artifact listener failed"),
            () = async {
                while !*shutdown.borrow() {
                    if shutdown.changed().await.is_err() { break; }
                }
            } => {
                let _ = stop.send(());
                serving.await.map_err(|_| "protected artifact listener failed")
            }
        }
    }
}
