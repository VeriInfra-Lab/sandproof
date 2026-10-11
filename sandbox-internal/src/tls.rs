use crate::TransportError;
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
    server::WebPkiClientVerifier,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Public certificate digest, registered out of band for an exact service role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CertificatePin([u8; 32]);

impl CertificatePin {
    #[must_use]
    pub fn from_certificate(certificate: &CertificateDer<'_>) -> Self {
        Self(Sha256::digest(certificate.as_ref()).into())
    }
}

/// Trusted in-memory identity. Deliberately no Debug/Serialize of private keys.
pub struct TlsIdentity {
    pub chain: Vec<CertificateDer<'static>>,
    pub private_key: PrivateKeyDer<'static>,
}

#[derive(Clone)]
pub struct ServerTls {
    pub(crate) config: Arc<ServerConfig>,
    peer_pins: Vec<CertificatePin>,
}

impl ServerTls {
    /// Roots verify chain/time/client usage; pins additionally bind center role.
    /// Rotation permits at most two protected, explicitly approved leaf pins.
    /// # Errors
    /// Rejects empty/invalid trust, identity and peer registration.
    pub fn new(
        identity: TlsIdentity,
        roots: Vec<CertificateDer<'static>>,
        peer_pins: Vec<CertificatePin>,
    ) -> Result<Self, TransportError> {
        validate_material(&identity, &peer_pins)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = WebPkiClientVerifier::builder_with_provider(
            Arc::new(root_store(roots)?),
            Arc::clone(&provider),
        )
        .build()
        .map_err(|_| TransportError::Configuration)?;
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| TransportError::Configuration)?
            .with_client_cert_verifier(verifier)
            .with_single_cert(identity.chain, identity.private_key)
            .map_err(|_| TransportError::Configuration)?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        // Every new connection revalidates the certificate and current pin.
        config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
        config.send_tls13_tickets = 0;
        config.max_early_data_size = 0;
        Ok(Self {
            config: Arc::new(config),
            peer_pins,
        })
    }

    pub(crate) fn verify_peer(
        &self,
        certificates: Option<&[CertificateDer<'_>]>,
    ) -> Result<(), TransportError> {
        verify_pin(certificates, &self.peer_pins)
    }
}

#[derive(Clone)]
pub struct ClientTls {
    pub(crate) config: Arc<ClientConfig>,
    peer_pins: Vec<CertificatePin>,
}

impl ClientTls {
    /// Roots and DNS name verify server validity; pins bind exact host role.
    /// # Errors
    /// Rejects empty/invalid trust, identity and peer registration.
    pub fn new(
        identity: TlsIdentity,
        roots: Vec<CertificateDer<'static>>,
        peer_pins: Vec<CertificatePin>,
    ) -> Result<Self, TransportError> {
        validate_material(&identity, &peer_pins)?;
        let mut config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .map_err(|_| TransportError::Configuration)?
                .with_root_certificates(root_store(roots)?)
                .with_client_auth_cert(identity.chain, identity.private_key)
                .map_err(|_| TransportError::Configuration)?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config.enable_early_data = false;
        config.resumption = rustls::client::Resumption::disabled();
        Ok(Self {
            config: Arc::new(config),
            peer_pins,
        })
    }

    pub(crate) fn verify_peer(
        &self,
        certificates: Option<&[CertificateDer<'_>]>,
    ) -> Result<(), TransportError> {
        verify_pin(certificates, &self.peer_pins)
    }
}

fn validate_material(
    identity: &TlsIdentity,
    pins: &[CertificatePin],
) -> Result<(), TransportError> {
    if identity.chain.is_empty()
        || identity.chain.len() > 8
        || !(1..=2).contains(&pins.len())
        || (pins.len() == 2 && pins[0] == pins[1])
        || identity.chain.iter().any(|cert| cert.len() > 64 * 1024)
        || identity.private_key.secret_der().len() > 64 * 1024
    {
        Err(TransportError::Configuration)
    } else {
        Ok(())
    }
}

fn root_store(roots: Vec<CertificateDer<'static>>) -> Result<RootCertStore, TransportError> {
    if roots.is_empty() || roots.len() > 8 || roots.iter().any(|root| root.len() > 64 * 1024) {
        return Err(TransportError::Configuration);
    }
    let mut store = RootCertStore::empty();
    for root in roots {
        store.add(root).map_err(|_| TransportError::Configuration)?;
    }
    Ok(store)
}

fn verify_pin(
    certificates: Option<&[CertificateDer<'_>]>,
    pins: &[CertificatePin],
) -> Result<(), TransportError> {
    if certificates
        .and_then(|chain| chain.first())
        .is_some_and(|leaf| pins.contains(&CertificatePin::from_certificate(leaf)))
    {
        Ok(())
    } else {
        Err(TransportError::Authentication)
    }
}
