//! Protected center-only assembly inputs. No listener, Redis initialization,
//! VM execution or placeholder template/build provider is installed here.
use crate::business::{MetadataBudget, RegistryMetadata, TemplateRegistration};
use crate::{CenterConfig, NodeObservationPolicy, PublicTemplate, RegisteredHost};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sandproof_api::{
    ApiCredential,
    admission::{RequestAdmission, RequestAdmissionLimits},
};
use sandproof_center_core::{Bounds, CenterState, NodeRegistration, TenantBudget};
use sandproof_center_store::{
    ControlState, RedisEndpoint, StoreConfig, authority::AuthorityState, business::BusinessEnvelope,
};
use sandproof_controller_service::{
    DEFAULT_TIMEOUT_SECS, template_registry::RegistryConfig, templates::TemplateCatalog,
};
use sandproof_data_transport::TransferLimits;
use sandproof_internal::{
    CertificatePin, ClientTls, InternalClient, Limits, RegisteredEndpoint, TlsIdentity,
    check_protected_directory, read_protected,
};
use sandproof_protocol::DeploymentId;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    net::{SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const CONFIG_BYTES: usize = 1024 * 1024;
const TLS_FILE_BYTES: usize = 64 * 1024;
mod artifacts;
mod builds;
pub use artifacts::{ArtifactAssembly, ArtifactPeer};
pub use builds::PrivateBuildBoundary;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssemblyError {
    ProtectedInput,
    InvalidConfiguration,
    InvalidTls,
    IncompleteTemplateRegistration,
    InvalidPublicBoundary,
}
impl AssemblyError {
    /// A fixed diagnostic category without configuration, paths or credentials.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProtectedInput => "protected center input rejected",
            Self::InvalidConfiguration => "invalid center deployment configuration",
            Self::InvalidTls => "invalid registered center TLS material",
            Self::IncompleteTemplateRegistration => "incomplete trusted template registration",
            Self::InvalidPublicBoundary => "invalid public center boundary",
        }
    }
}
impl std::fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl std::error::Error for AssemblyError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialFile {
    tenant: u64,
    secret: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RedisFile {
    address: SocketAddrV4,
    username: String,
    password: String,
    max_block_bytes: usize,
    memory_bytes: u64,
    operation_timeout_ms: u64,
    writer_root: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsFile {
    certificate_chain_der: Vec<PathBuf>,
    private_key_der: PathBuf,
    trusted_roots_der: Vec<PathBuf>,
    max_connections: usize,
    handshake_timeout_ms: u64,
    request_timeout_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeFile {
    registration: NodeRegistration,
    address: SocketAddr,
    server_name: String,
    peer_certificates_der: Vec<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentFile {
    deployment_id: DeploymentId,
    bind: SocketAddr,
    domain: String,
    credentials: Vec<CredentialFile>,
    data_token_key: PathBuf,
    max_timeout_secs: u64,
    max_lifetime_secs: u64,
    startup_timeout_secs: u64,
    reply_timeout_ms: u64,
    poll_interval_ms: u64,
    expiry_scan_ms: u64,
    node_observation: NodeObservationPolicy,
    public_ipv4_enabled: bool,
    request_admission: RequestAdmissionLimits,
    data_transfer: TransferLimits,
    redis: RedisFile,
    tls: TlsFile,
    bounds: Bounds,
    tenants: Vec<TenantBudget>,
    nodes: Vec<NodeFile>,
    template_registry: RegistryConfig,
    metadata_budget: MetadataBudget,
    #[serde(deserialize_with = "explicit_private_builds")]
    private_builds: Option<builds::PrivateBuildFile>,
    #[serde(deserialize_with = "explicit_private_boundary")]
    private_build_boundary: Option<builds::PrivateBoundaryFile>,
    templates: TemplateCatalog,
    template_registrations: Vec<TemplateRegistration>,
    artifact_service: artifacts::ArtifactFile,
}

/// Validated ingredients only. No `Debug`/`Serialize` exposes user/Redis keys.
/// The complete mutable template publication/build provider and remote data
/// gateway remain mandatory dependencies of the eventual executable assembly.
pub struct CenterAssemblyInputs {
    pub application: CenterConfig,
    pub bind: SocketAddr,
    pub credentials: Vec<ApiCredential>,
    pub admission: Arc<RequestAdmission>,
    pub transfers: TransferLimits,
    pub token_key: Vec<u8>,
    pub redis_endpoint: RedisEndpoint,
    pub store: StoreConfig,
    pub writer_root: PathBuf,
    /// An initial registration for EXPLICIT provisioning, not a recovery state.
    /// Attaching an existing deployment must check its own Redis authority.
    pub provisioning: BusinessEnvelope<RegistryMetadata>,
    pub hosts: Vec<RegisteredHost>,
    pub expiry_scan_interval: Duration,
    pub node_observation: NodeObservationPolicy,
    pub artifacts: ArtifactAssembly,
    pub private_build_boundary: Option<PrivateBuildBoundary>,
}

fn certificate(path: &Path, owner: u32) -> Result<CertificateDer<'static>, AssemblyError> {
    read_protected(path, owner, TLS_FILE_BYTES)
        .map(CertificateDer::from)
        .map_err(|_| AssemblyError::ProtectedInput)
}

fn certificates(
    paths: &[PathBuf],
    owner: u32,
) -> Result<Vec<CertificateDer<'static>>, AssemblyError> {
    if paths.is_empty() || paths.len() > 4 {
        return Err(AssemblyError::InvalidTls);
    }
    paths.iter().map(|path| certificate(path, owner)).collect()
}

fn clients(
    file: &TlsFile,
    nodes: &[NodeFile],
    owner: u32,
) -> Result<Vec<RegisteredHost>, AssemblyError> {
    let chain = certificates(&file.certificate_chain_der, owner)?;
    let roots = certificates(&file.trusted_roots_der, owner)?;
    let key = PrivateKeyDer::try_from(
        read_protected(&file.private_key_der, owner, TLS_FILE_BYTES)
            .map_err(|_| AssemblyError::ProtectedInput)?,
    )
    .map_err(|_| AssemblyError::InvalidTls)?;
    let limits = Limits::new(
        file.max_connections,
        Duration::from_millis(file.handshake_timeout_ms),
        Duration::from_millis(file.request_timeout_ms),
    )
    .map_err(|_| AssemblyError::InvalidTls)?;
    let mut registered_pins = Vec::new();
    let center_pin = CertificatePin::from_certificate(&chain[0]);
    nodes
        .iter()
        .map(|node| {
            if node.peer_certificates_der.is_empty() || node.peer_certificates_der.len() > 2 {
                return Err(AssemblyError::InvalidTls);
            }
            let mut pins = Vec::new();
            for path in &node.peer_certificates_der {
                let pin = CertificatePin::from_certificate(&certificate(path, owner)?);
                if pin == center_pin || registered_pins.contains(&pin) || pins.contains(&pin) {
                    return Err(AssemblyError::InvalidTls);
                }
                pins.push(pin);
            }
            registered_pins.extend(pins.iter().copied());
            let tls = ClientTls::new(
                TlsIdentity {
                    chain: chain.clone(),
                    private_key: key.clone_key(),
                },
                roots.clone(),
                pins,
            )
            .map_err(|_| AssemblyError::InvalidTls)?;
            let endpoint = RegisteredEndpoint::new(
                node.address,
                node.server_name.clone(),
                node.registration.identity.clone(),
            )
            .map_err(|_| AssemblyError::InvalidConfiguration)?;
            Ok(RegisteredHost {
                identity: node.registration.identity.clone(),
                client: InternalClient::new(tls, endpoint, limits),
            })
        })
        .collect()
}

fn application_config(file: &DeploymentFile) -> Result<CenterConfig, AssemblyError> {
    let invalid = || AssemblyError::InvalidConfiguration;
    if !file.bind.ip().is_loopback()
        || file.bind.port() == 0
        || file.max_timeout_secs < DEFAULT_TIMEOUT_SECS
        || file.expiry_scan_ms == 0
        || file.expiry_scan_ms > 86_400_000
        || file.credentials.is_empty()
        || file.credentials.len() > usize::from(file.bounds.max_sandboxes)
        || file.nodes.is_empty()
        || !file
            .nodes
            .iter()
            .any(|node| node.registration.role == sandproof_center_core::NodeRole::Runtime)
        || file.nodes.len() > usize::from(file.bounds.max_nodes)
        || file.nodes.iter().any(|node| {
            node.registration.healthy_until_ms != 0
                || node.registration.free
                    != sandproof_protocol::internal::lifecycle::Resources::ZERO
                || !node.registration.templates.is_empty()
        })
        || file.bounds.operations_per_sandbox < 4
        || file
            .nodes
            .iter()
            .map(|n| n.address)
            .collect::<BTreeSet<_>>()
            .len()
            != file.nodes.len()
        || !file.template_registry.is_valid()
        || file.tls.max_connections <= file.data_transfer.max_transfers
    {
        return Err(invalid());
    }
    let application = CenterConfig {
        domain: file.domain.clone(),
        max_timeout_secs: file.max_timeout_secs,
        max_lifetime_secs: file.max_lifetime_secs,
        startup_timeout_secs: file.startup_timeout_secs,
        reply_timeout: Duration::from_millis(file.reply_timeout_ms),
        poll_interval: Duration::from_millis(file.poll_interval_ms),
        public_ipv4_enabled: file.public_ipv4_enabled,
    };
    application.validate().map_err(|_| invalid())?;
    Ok(application)
}

fn validate_templates(
    entries: &[TemplateRegistration],
    catalog: &TemplateCatalog,
    nodes: &[NodeFile],
    max_templates: usize,
) -> Result<(), AssemblyError> {
    catalog
        .validate()
        .map_err(|_| AssemblyError::IncompleteTemplateRegistration)?;
    if entries.len() != catalog.templates.len()
        || entries.len() > max_templates
        || entries
            .iter()
            .map(|t| &t.template_id)
            .collect::<BTreeSet<_>>()
            .len()
            != entries.len()
    {
        return Err(AssemblyError::IncompleteTemplateRegistration);
    }
    let templates: Vec<_> = entries
        .iter()
        .map(|t| {
            let binding = catalog
                .templates
                .iter()
                .find(|binding| binding.template_id == t.template_id)
                .ok_or(sandproof_api::PortError::Invalid)?;
            PublicTemplate::from_binding(binding.clone(), t.revision.clone())
        })
        .collect::<Result<_, _>>()
        .map_err(|_| AssemblyError::IncompleteTemplateRegistration)?;
    if templates
        .iter()
        .map(|t| t.revision().as_str())
        .collect::<BTreeSet<_>>()
        .len()
        != templates.len()
        || nodes.iter().any(|n| {
            n.registration.templates.iter().any(|t| {
                !templates
                    .iter()
                    .any(|known| known.revision() == &t.revision && known.digest() == t.digest)
            })
        })
    {
        return Err(AssemblyError::IncompleteTemplateRegistration);
    }
    Ok(())
}

impl CenterAssemblyInputs {
    /// Load root/service-account-protected configuration and DER TLS material.
    /// The center cannot accept host `database`, jailer, UID or VM configuration;
    /// it does not infer defaults from a missing host mode or silently initialize.
    /// # Errors
    /// Fails closed on unsafe files, unknown fields, contradictory bounds,
    /// incomplete immutable template bindings, credentials or peer registration.
    pub fn load(path: &Path, owner_uid: u32) -> Result<Self, AssemblyError> {
        let bytes = read_protected(path, owner_uid, CONFIG_BYTES)
            .map_err(|_| AssemblyError::ProtectedInput)?;
        let file: DeploymentFile =
            serde_json::from_slice(&bytes).map_err(|_| AssemblyError::InvalidConfiguration)?;
        Self::validate(file, owner_uid)
    }

    fn validate(file: DeploymentFile, owner: u32) -> Result<Self, AssemblyError> {
        let invalid = || AssemblyError::InvalidConfiguration;
        let application = application_config(&file)?;
        let private_build_boundary = private_boundary(&file, owner)?;
        let private_builds = file
            .private_builds
            .as_ref()
            .map(|build| {
                build.resolve(
                    &file.templates,
                    &file.template_registrations,
                    file.credentials[0].tenant,
                )
            })
            .transpose()?;
        file.node_observation
            .validate(Duration::from_millis(file.expiry_scan_ms))
            .map_err(|_| invalid())?;
        let artifacts = artifacts::load(
            &file.artifact_service,
            &file.nodes,
            &file.template_registry,
            owner,
        )?;
        if artifacts.bind == file.bind
            || file.nodes.iter().any(|node| node.address == artifacts.bind)
        {
            return Err(invalid());
        }
        file.data_transfer.validate().map_err(|_| invalid())?;
        let admission = RequestAdmission::new(file.request_admission).map_err(|_| invalid())?;
        let credentials = credentials(file.credentials, &file.tenants)?;
        let token_key = read_protected(&file.data_token_key, owner, 4096)
            .map_err(|_| AssemblyError::ProtectedInput)?;
        if token_key.len() < 32 {
            return Err(invalid());
        }
        let registration = CenterState::new(
            file.deployment_id.clone(),
            file.bounds,
            file.nodes.iter().map(|n| n.registration.clone()).collect(),
            file.tenants,
        )
        .map_err(|_| invalid())?;
        let store = StoreConfig::new(
            file.deployment_id,
            file.redis.max_block_bytes,
            file.redis.memory_bytes,
            Duration::from_millis(file.redis.operation_timeout_ms),
        )
        .map_err(|_| invalid())?;
        if registration.bounds().required_block_bytes() > file.redis.max_block_bytes {
            return Err(invalid());
        }
        let redis_endpoint =
            RedisEndpoint::new(file.redis.address, file.redis.username, file.redis.password)
                .map_err(|_| invalid())?;
        check_protected_directory(&file.redis.writer_root, owner)
            .map_err(|_| AssemblyError::ProtectedInput)?;
        check_registry_directories(&file.template_registry, owner)?;
        validate_templates(
            &file.template_registrations,
            &file.templates,
            &file.nodes,
            file.template_registry.max_templates_global,
        )?;
        let metadata = RegistryMetadata::new(
            file.templates,
            file.template_registrations,
            file.template_registry,
            file.metadata_budget,
            private_builds,
        )
        .map_err(|_| invalid())?;
        let provisioning = BusinessEnvelope {
            center: registration,
            metadata,
        };
        // Provisioning and recovery use exactly the same whole-envelope bound.
        // Static descriptors cannot silently bypass future validation receipts.
        if !provisioning.is_valid()
            || provisioning.required_block_bytes() > file.redis.max_block_bytes
        {
            return Err(invalid());
        }
        let hosts = clients(&file.tls, &file.nodes, owner)?;
        Ok(Self {
            application,
            bind: file.bind,
            credentials,
            admission,
            transfers: file.data_transfer,
            token_key,
            redis_endpoint,
            store,
            writer_root: file.redis.writer_root,
            provisioning,
            hosts,
            expiry_scan_interval: Duration::from_millis(file.expiry_scan_ms),
            node_observation: file.node_observation,
            artifacts,
            private_build_boundary,
        })
    }
}

fn explicit_private_builds<'de, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<Option<builds::PrivateBuildFile>, D::Error> {
    Option::deserialize(input)
}
fn explicit_private_boundary<'de, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<Option<builds::PrivateBoundaryFile>, D::Error> {
    Option::deserialize(input)
}
fn private_boundary(
    file: &DeploymentFile,
    owner: u32,
) -> Result<Option<PrivateBuildBoundary>, AssemblyError> {
    if file.private_builds.is_some() != file.private_build_boundary.is_some() {
        return Err(AssemblyError::InvalidConfiguration);
    }
    let value = file
        .private_build_boundary
        .as_ref()
        .map(|boundary| builds::load(boundary, owner))
        .transpose()?;
    if value.as_ref().is_some_and(|boundary| {
        [boundary.bind, boundary.backend_address]
            .iter()
            .any(|address| {
                *address == file.bind
                    || *address == file.artifact_service.bind
                    || file.nodes.iter().any(|node| node.address == *address)
            })
    }) {
        return Err(AssemblyError::InvalidConfiguration);
    }
    Ok(value)
}

fn credentials(
    input: Vec<CredentialFile>,
    tenants: &[TenantBudget],
) -> Result<Vec<ApiCredential>, AssemblyError> {
    let credentials: Vec<_> = input
        .into_iter()
        .map(|c| ApiCredential {
            tenant: c.tenant.to_string(),
            secret: c.secret,
        })
        .collect();
    sandproof_api::validate_credentials(&credentials)
        .map_err(|_| AssemblyError::InvalidConfiguration)?;
    if credentials
        .iter()
        .any(|c| c.tenant == "0" || !tenants.iter().any(|t| t.tenant.as_str() == c.tenant))
    {
        return Err(AssemblyError::InvalidConfiguration);
    }
    Ok(credentials)
}

fn check_registry_directories(registry: &RegistryConfig, owner: u32) -> Result<(), AssemblyError> {
    for directory in [
        &registry.root,
        &registry.root.join("inbox"),
        &registry.root.join("artifacts"),
    ] {
        check_protected_directory(directory, owner).map_err(|_| AssemblyError::ProtectedInput)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
