use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits};
use sandproof_center_core::{
    AvailableTemplate, Bounds, CenterState, NodeMode, NodeRegistration, TenantBudget,
};
use sandproof_center_store::{
    RedisEndpoint, RedisLedger, StoreConfig, WriterLease, authority::CenterAuthority,
};
use sandproof_controller_service::{
    AdmissionLimits, ControllerJournal, InternalInboxConfig, InternalTemplate, MaintenanceConfig,
    NeverCrash,
    application::{ApplicationConfig, ControllerApplication, TemplateArtifacts},
    templates::{ComputeLimits, TemplateBinding},
};
use sandproof_host_service::{HostFacade, data_gateway::TransferLimits};
use sandproof_internal::{
    CertificatePin, ClientTls, InternalClient, InternalServer, Limits, RegisteredEndpoint,
    ServerTls, TlsIdentity,
};
use sandproof_node::{LinuxWorker, NodeConfig, NodeHost, PhysicalProfileLease};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{CreateBinding, Intent, LifecycleFact, Resources},
};
use serde::Deserialize;
use std::{
    error::Error,
    fs,
    net::{SocketAddr, SocketAddrV4},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "../../../sandbox-internal/tests/support/tls.rs"]
mod tls_fixture;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn checked<T, E: std::fmt::Debug>(value: std::result::Result<T, E>) -> Result<T> {
    value.map_err(|e| format!("{e:?}").into())
}
fn now() -> Result<u64> {
    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn certificate(root: &Path, name: &str) -> Result<CertificateDer<'static>> {
    Ok(fs::read(root.join(format!("{name}.der")))?.into())
}
fn identity(root: &Path, name: &str) -> Result<TlsIdentity> {
    Ok(TlsIdentity {
        chain: vec![certificate(root, name)?],
        private_key: PrivatePkcs8KeyDer::from(fs::read(root.join(format!("{name}.key")))?).into(),
    })
}
fn limits() -> Result<Limits> {
    Ok(Limits::new(
        4,
        Duration::from_secs(2),
        Duration::from_secs(10),
    )?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfig {
    node: NodeConfig,
    maintenance: MaintenanceConfig,
    database: PathBuf,
    identity: NodeIdentity,
    template: TemplateBinding,
    template_digest: [u8; 32],
    template_revision: sandproof_protocol::ResourceId,
    tls: PathBuf,
    bind: SocketAddr,
    relay: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CenterConfig {
    identity: NodeIdentity,
    binding: CreateBinding,
    redis: SocketAddrV4,
    password: String,
    tls: PathBuf,
    host: SocketAddr,
    root: PathBuf,
}

fn application(cfg: &HostConfig) -> Result<Arc<ControllerApplication<NodeHost>>> {
    let bounded = cfg.node.bounded_host.clone();
    let profile = bounded
        .physical_storage
        .as_ref()
        .ok_or("missing physical profile")?;
    let pool = profile.pools.first().ok_or("missing runtime pool")?;
    let shape = cfg.template.resources;
    let storage = PhysicalProfileLease::activate(
        profile,
        &cfg.node.namespace,
        &cfg.node.firecracker,
        &cfg.node.kernel,
    )?;
    let worker = LinuxWorker::new(cfg.node.clone())?;
    let inventory = worker.inventory()?;
    if !inventory.slots.is_empty()
        || !inventory.unbound_claims.is_empty()
        || !inventory.unbound_networks.is_empty()
        || !inventory.unbound_cgroups.is_empty()
    {
        return Err("acceptance requires a fresh empty host inventory".into());
    }
    let mut journal = ControllerJournal::open_complete(
        &cfg.database,
        &cfg.node.namespace,
        bounded.clone(),
        cfg.maintenance.clone(),
        now()?,
    )?;
    journal.initialize_internal_inbox(
        InternalInboxConfig {
            node: cfg.identity.clone(),
            max_sandboxes: 1,
            operations_per_sandbox: 2,
            max_options_bytes: 1024,
        },
        &mut NeverCrash,
    )?;
    let node = NodeHost::new(worker, storage, cfg.relay.clone());
    checked(node.validate_template(&cfg.template))?;
    Ok(Arc::new(ControllerApplication::new(
        journal,
        node,
        ApplicationConfig {
            public_ipv4_enabled: false,
            limits: AdmissionLimits {
                max_active_global: 1,
                max_active_per_tenant: 1,
                max_disk_bytes_global: pool.rootfs_budget_bytes + pool.cleanup_reserve_bytes,
                max_disk_bytes_per_tenant: shape.disk_bytes,
                cleanup_reserve_bytes: pool.cleanup_reserve_bytes,
                max_timeout_secs: 300,
                max_lifetime_secs: 600,
                startup_timeout_secs: 180,
            },
            disk_bytes: shape.disk_bytes,
            file_policy_digest: cfg.node.file_policy_digest,
            cpu_count: u32::from(shape.cpu_count),
            memory_mb: shape.memory_mib,
            disk_size_mb: u32::try_from(shape.disk_bytes / (1024 * 1024))?,
            envd_version: cfg.template.envd_version.clone(),
            domain: "example.invalid".into(),
        },
    )))
}

async fn host(path: &Path) -> Result<()> {
    let cfg: HostConfig = serde_json::from_slice(&fs::read(path)?)?;
    let app = application(&cfg)?;
    let shape = cfg.template.resources;
    let facade = HostFacade::new(
        Arc::clone(&app),
        TransferLimits {
            file_upload_bytes: 8 * 1024 * 1024,
            file_download_bytes: 8 * 1024 * 1024,
            command_input_bytes: 1024 * 1024,
            rpc_request_bytes: 64 * 1024,
            rpc_response_bytes: 1024 * 1024,
            max_transfers: 3,
            max_transfers_per_vm: 2,
            control_reserve: 1,
            request_idle_seconds: 10,
            file_idle_seconds: 10,
            write_idle_seconds: 10,
        },
        RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 4,
            control_reserve: 1,
        })?,
    )?;
    let inbox = facade.lifecycle();
    let compute = ComputeLimits {
        max_vcpus_global: u64::from(shape.cpu_count),
        max_vcpus_per_tenant: u64::from(shape.cpu_count),
        max_memory_bytes_global: shape.host_memory_bytes,
        max_memory_bytes_per_tenant: shape.host_memory_bytes,
    };
    checked(inbox.recover().await)?;
    let server = InternalServer::new(
        ServerTls::new(
            identity(&cfg.tls, "host")?,
            vec![certificate(&cfg.tls, "ca")?],
            vec![CertificatePin::from_certificate(&certificate(
                &cfg.tls, "center",
            )?)],
        )?,
        cfg.identity.clone(),
        limits()?,
        sandproof_internal::diagnostic::RejectTemplateCache::new(facade),
    );
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    write(&cfg.tls.join("listening"), b"authenticated host listening")?;
    let (stop, shutdown) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move { server.serve(listener, shutdown).await });
    let until = tokio::time::Instant::now() + Duration::from_mins(10);
    let execution = async {
        loop {
            if tokio::time::Instant::now() >= until {
                return Err("host acceptance deadline; resources may remain".into());
            }
            for request in checked(app.internal_pending(&cfg.identity))? {
                let template = (request.intent == Intent::Create).then(|| InternalTemplate {
                    revision: cfg.template_revision.clone(),
                    digest: cfg.template_digest,
                    registered: cfg.template.clone(),
                    local: cfg.template.clone(),
                });
                match inbox.advance(request.clone(), template, compute).await {
                    Ok(response)
                        if request.intent == Intent::Destroy
                            && response.fact == LifecycleFact::Cleaned =>
                    {
                        // Allow the center to fetch the DURABLE cached fact. The
                        // parent terminates only after exact residual checks.
                        write(&cfg.tls.join("cleaned"), b"original host cleanup confirmed")?;
                        return Ok(());
                    }
                    Ok(_)
                    | Err(sandproof_controller_service::application::ApplicationError::Busy) => (),
                    Err(error) => eprintln!("host advancement: {error:?}"),
                }
            }
            // Original local expiry is independent of center reachability.
            let _ = inbox.recover().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    .await;
    if execution.is_ok() {
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let _ = stop.send(());
    task.await??;
    execution
}

async fn await_fact(
    center: &mut CenterAuthority,
    client: &Arc<InternalClient>,
    ready: bool,
) -> Result<sandproof_protocol::internal::lifecycle::LifecycleResponse> {
    let until = tokio::time::Instant::now() + Duration::from_mins(3);
    loop {
        for dispatch in checked(center.dispatch_batch())? {
            let mut transport = {
                let client = Arc::clone(client);
                move |request| {
                    let client = Arc::clone(&client);
                    async move { client.lifecycle(&request).await }
                }
            };
            if let Ok(response) = dispatch.send(&mut transport).await {
                checked(center.apply_response(&response))?;
                if matches!(response.fact, LifecycleFact::Ready { .. }) && ready
                    || response.fact == LifecycleFact::Cleaned && !ready
                {
                    return Ok(response);
                }
                if response.fact == LifecycleFact::NotAccepted {
                    return Err("host refused create".into());
                }
            }
        }
        if tokio::time::Instant::now() >= until {
            return Err("durable fact deadline; reservation remains held".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn guest_health(
    client: &InternalClient,
    response: &sandproof_protocol::internal::lifecycle::LifecycleResponse,
    root: &Path,
) -> Result<()> {
    // Same registered mTLS -> original host permit -> durably launched
    // namespace relay -> real guest envd. No native guest secret at center.
    let LifecycleFact::Ready { max_end_at_ms, .. } = response.fact else {
        return Err("missing original Ready lifetime".into());
    };
    let deadline = tokio::time::Instant::now()
        + Duration::from_millis(
            max_end_at_ms
                .checked_sub(now()?)
                .ok_or("original maximum lifetime reached")?,
        );
    let health = client
        .stream_data(
            &response.identity,
            Request::get("/health").body(Body::empty())?,
            Arc::new(()),
            deadline,
        )
        .await?;
    let status = health.status();
    let _ =
        tokio::time::timeout(Duration::from_secs(10), to_bytes(health.into_body(), 4096)).await??;
    write(
        &root.join("guest-health.json"),
        &serde_json::to_vec(&serde_json::json!({
            "identity": response.identity,
            "path": "/health",
            "status": status.as_u16(),
        }))?,
    )?;
    require_guest_health(status)
}

fn require_guest_health(status: axum::http::StatusCode) -> Result<()> {
    // The original pinned envd health contract returns 204, not a rewritten
    // success code from the relay. Keep errors observable without guest data.
    if status != axum::http::StatusCode::NO_CONTENT {
        return Err(format!("real authenticated guest health status {}", status.as_u16()).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_keeps_the_original_envd_status_contract() {
        assert!(require_guest_health(axum::http::StatusCode::NO_CONTENT).is_ok());
        for status in [200, 401, 404, 429, 502, 503] {
            assert!(
                require_guest_health(axum::http::StatusCode::from_u16(status).unwrap()).is_err()
            );
        }
    }
}

async fn center(path: &Path) -> Result<()> {
    let cfg: CenterConfig = serde_json::from_slice(&fs::read(path)?)?;
    let current = now()?;
    let resources = cfg.binding.resources;
    let state = checked(CenterState::new(
        cfg.identity.deployment_id.clone(),
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 1,
            max_sandboxes: 1,
            operations_per_sandbox: 2,
            max_options_bytes: 1024,
            platform: resources,
        },
        vec![NodeRegistration {
            role: sandproof_center_core::NodeRole::Runtime,
            identity: cfg.identity.clone(),
            limit: resources,
            mode: NodeMode::Accepting,
            healthy_until_ms: current + 600_000,
            free: resources,
            templates: vec![AvailableTemplate {
                revision: cfg.binding.template_revision.clone(),
                digest: cfg.binding.template_digest,
            }],
            protocol_version: 1,
        }],
        vec![TenantBudget {
            tenant: "1".try_into()?,
            limit: resources,
        }],
    ))?;
    let config = StoreConfig::new(
        cfg.identity.deployment_id.clone(),
        state.bounds().required_block_bytes(),
        64 << 20,
        Duration::from_secs(5),
    )?;
    let endpoint = RedisEndpoint::new(cfg.redis, "default".into(), cfg.password)?;
    let lease = WriterLease::acquire(&cfg.root, cfg.identity.deployment_id.clone())?;
    let (ledger, _) = RedisLedger::initialize(endpoint, config, lease, &state)?;
    let mut center = checked(CenterAuthority::attach(ledger))?;
    let client = Arc::new(InternalClient::new(
        ClientTls::new(
            identity(&cfg.tls, "center")?,
            vec![certificate(&cfg.tls, "ca")?],
            vec![CertificatePin::from_certificate(&certificate(
                &cfg.tls, "host",
            )?)],
        )?,
        RegisteredEndpoint::new(cfg.host, "host.test".into(), cfg.identity)?,
        limits()?,
    ));
    let mut binding = cfg.binding;
    binding.startup_deadline_ms = current + 150_000;
    let tenant = "1".try_into()?;
    let create = checked(center.create(&tenant, binding, current))?;
    let running = async {
        let response = await_fact(&mut center, &client, true).await?;
        if checked(center.state())?.reserved(None, None) != Some(resources) {
            return Err("running reservation missing".into());
        }
        write(
            &cfg.root.join("ready.json"),
            &serde_json::to_vec(&response)?,
        )?;
        guest_health(&client, &response, &cfg.root).await?;
        let until = tokio::time::Instant::now() + Duration::from_secs(30);
        while !cfg.root.join("release").exists() {
            if tokio::time::Instant::now() >= until {
                return Err("parent did not confirm live physical identity".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<_, Box<dyn Error>>(())
    }
    .await;
    // Even a failed Ready assertion must request ORIGINAL durable destruction.
    checked(center.destroy(&tenant, &create.identity.sandbox_id))?;
    let cleaned = await_fact(&mut center, &client, false).await?;
    if checked(center.state())?.reserved(None, None) != Some(Resources::ZERO) {
        return Err("cleaned reservation still held".into());
    }
    write(
        &cfg.root.join("cleaned.json"),
        &serde_json::to_vec(&cleaned)?,
    )?;
    running
}

pub fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: kvm_lifecycle certificates|host|center PRIVATE_CONFIG_OR_ROOT".into());
    }
    let path = Path::new(&args[2]);
    if args[1] == "certificates" {
        let certs = tls_fixture::certificates();
        write(&path.join("ca.der"), certs.ca.der())?;
        for (name, identity) in [("center", certs.center), ("host", certs.host)] {
            write(
                &path.join(format!("{name}.der")),
                identity.certificate.der(),
            )?;
            write(
                &path.join(format!("{name}.key")),
                &identity.key.serialize_der(),
            )?;
        }
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    if args[1] == "host" {
        runtime.block_on(host(path))
    } else if args[1] == "center" {
        runtime.block_on(center(path))
    } else {
        Err("unknown acceptance role".into())
    }
}
