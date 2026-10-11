//! Protected artifact-role TLS inputs; no file body is read while loading.
use super::{
    AssemblyError, NodeFile, RegistryConfig, TLS_FILE_BYTES, certificates, read_protected,
};
use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
};
use sandproof_internal::{CertificatePin, Limits, ServerTls, TlsIdentity};
use sandproof_protocol::internal::NodeIdentity;
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactTls {
    #[serde(rename = "certificate_chain_der")]
    chain: Vec<PathBuf>,
    #[serde(rename = "private_key_der")]
    key: PathBuf,
    #[serde(rename = "trusted_roots_der")]
    roots: Vec<PathBuf>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtifactFile {
    pub(super) bind: SocketAddr,
    server_name: String,
    tls: ArtifactTls,
    max_connections: usize,
    handshake_timeout_ms: u64,
    request_timeout_ms: u64,
    stream_timeout_ms: u64,
}
#[derive(Clone)]
pub struct ArtifactPeer {
    pub node: NodeIdentity,
    pub pins: Vec<CertificatePin>,
}
/// No Debug/Serialize exposes TLS material. The original config root belongs
/// to the CENTER artifact account, never the host cache, jail or `SQLite` root.
pub struct ArtifactAssembly {
    pub bind: SocketAddr,
    pub tls: ServerTls,
    pub peers: Vec<ArtifactPeer>,
    pub limits: Limits,
    pub stream_timeout: Duration,
    pub registry: RegistryConfig,
    pub owner_uid: u32,
}
pub(super) fn load(
    file: &ArtifactFile,
    nodes: &[NodeFile],
    registry: &RegistryConfig,
    owner: u32,
) -> Result<ArtifactAssembly, AssemblyError> {
    if file.bind.port() == 0
        || file.bind.ip().is_unspecified()
        || file.bind.ip().is_multicast()
        || file.stream_timeout_ms < file.request_timeout_ms
        || file.stream_timeout_ms > 86_400_000
    {
        return Err(AssemblyError::InvalidConfiguration);
    }
    let limits = Limits::new(
        file.max_connections,
        Duration::from_millis(file.handshake_timeout_ms),
        Duration::from_millis(file.request_timeout_ms),
    )
    .map_err(|_| AssemblyError::InvalidConfiguration)?;
    let chain = certificates(&file.tls.chain, owner)?;
    let roots = certificates(&file.tls.roots, owner)?;
    validate_server_identity(&chain, &roots, &file.server_name)?;
    let key = PrivateKeyDer::try_from(
        read_protected(&file.tls.key, owner, TLS_FILE_BYTES)
            .map_err(|_| AssemblyError::ProtectedInput)?,
    )
    .map_err(|_| AssemblyError::InvalidTls)?;
    let mut all_pins = Vec::new();
    let mut peers = Vec::new();
    for node in nodes {
        let pins: Vec<_> = certificates(&node.peer_certificates_der, owner)?
            .iter()
            .map(CertificatePin::from_certificate)
            .collect();
        if pins.len() > 2
            || pins.iter().any(|pin| {
                all_pins.contains(pin) || *pin == CertificatePin::from_certificate(&chain[0])
            })
        {
            return Err(AssemblyError::InvalidTls);
        }
        all_pins.extend(pins.iter().copied());
        peers.push(ArtifactPeer {
            node: node.registration.identity.clone(),
            pins,
        });
    }
    let tls = ServerTls::for_registered_hosts(
        TlsIdentity {
            chain,
            private_key: key,
        },
        roots,
        all_pins,
    )
    .map_err(|_| AssemblyError::InvalidTls)?;
    Ok(ArtifactAssembly {
        bind: file.bind,
        tls,
        peers,
        limits,
        stream_timeout: Duration::from_millis(file.stream_timeout_ms),
        registry: registry.clone(),
        owner_uid: owner,
    })
}

fn validate_server_identity(
    chain: &[CertificateDer<'static>],
    roots: &[CertificateDer<'static>],
    name: &str,
) -> Result<(), AssemblyError> {
    let mut trust = RootCertStore::empty();
    for root in roots {
        trust
            .add(root.clone())
            .map_err(|_| AssemblyError::InvalidTls)?;
    }
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(trust),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .map_err(|_| AssemblyError::InvalidTls)?;
    let name = ServerName::try_from(name).map_err(|_| AssemblyError::InvalidTls)?;
    verifier
        .verify_server_cert(&chain[0], &chain[1..], &name, &[], UnixTime::now())
        .map_err(|_| AssemblyError::InvalidTls)?;
    Ok(())
}
