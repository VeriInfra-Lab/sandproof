//! Same protected material loader used by internal transport trust validation.
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sandproof_internal::{CertificatePin, ClientTls, ServerTls, TlsIdentity, read_protected};
use serde::Deserialize;
use std::{error::Error, path::PathBuf};

const MAX_TLS_BYTES: usize = 64 * 1024;

fn certificates(paths: &[PathBuf]) -> Result<Vec<CertificateDer<'static>>, Box<dyn Error>> {
    paths
        .iter()
        .map(|path| {
            Ok(CertificateDer::from(read_protected(
                path,
                0,
                MAX_TLS_BYTES,
            )?))
        })
        .collect()
}
fn identity(chain: &[PathBuf], key: &std::path::Path) -> Result<TlsIdentity, Box<dyn Error>> {
    Ok(TlsIdentity {
        chain: certificates(chain)?,
        private_key: PrivateKeyDer::try_from(read_protected(key, 0, MAX_TLS_BYTES)?)
            .map_err(|_| "invalid protected host TLS key")?,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtifactTlsFiles {
    identity_chain: Vec<PathBuf>,
    private_key: PathBuf,
    roots: Vec<PathBuf>,
    artifact_leaves: Vec<PathBuf>,
}
impl ArtifactTlsFiles {
    pub(super) fn validate(&self) -> Result<(), Box<dyn Error>> {
        if !(1..=8).contains(&self.identity_chain.len())
            || !(1..=8).contains(&self.roots.len())
            || !(1..=2).contains(&self.artifact_leaves.len())
        {
            return Err("invalid protected artifact TLS registration".into());
        }
        Ok(())
    }
    pub(super) fn load(&self) -> Result<ClientTls, Box<dyn Error>> {
        self.validate()?;
        Ok(ClientTls::new(
            identity(&self.identity_chain, &self.private_key)?,
            certificates(&self.roots)?,
            certificates(&self.artifact_leaves)?
                .iter()
                .map(CertificatePin::from_certificate)
                .collect(),
        )?)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TlsFiles {
    identity_chain: Vec<PathBuf>,
    private_key: PathBuf,
    roots: Vec<PathBuf>,
    center_leaves: Vec<PathBuf>,
}
impl TlsFiles {
    pub(super) fn center_registration(&self, center: sandproof_protocol::lifecycle::CenterIdentity, node: sandproof_protocol::internal::NodeIdentity) -> Result<sandproof_internal::lifecycle::CenterPeerRegistration, Box<dyn Error>> {
        self.validate()?;
        Ok(sandproof_internal::lifecycle::CenterPeerRegistration::new(center, node,
            certificates(&self.center_leaves)?.iter().map(CertificatePin::from_certificate).collect())?)
    }
    pub(super) fn validate(&self) -> Result<(), Box<dyn Error>> {
        if !(1..=8).contains(&self.identity_chain.len())
            || !(1..=8).contains(&self.roots.len())
            || !(1..=2).contains(&self.center_leaves.len())
        {
            return Err("invalid protected host TLS registration".into());
        }
        Ok(())
    }
    pub(super) fn load(&self) -> Result<ServerTls, Box<dyn Error>> {
        self.validate()?;
        let pins = certificates(&self.center_leaves)?
            .iter()
            .map(CertificatePin::from_certificate)
            .collect();
        Ok(ServerTls::new(
            identity(&self.identity_chain, &self.private_key)?,
            certificates(&self.roots)?,
            pins,
        )?)
    }
}
