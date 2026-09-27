#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod gateway;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("sandproof-server requires Linux/KVM");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::gateway;
    use std::{
        error::Error,
        fs,
        net::SocketAddr,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        sync::Arc,
        time::Duration,
    };

    use sandbox_controller_core as core;
    use sandproof_api::{
        ApiCredential, PortError,
        managed::{HostAdapter, ManagedConfig, ManagedPort},
        router,
    };
    use sandproof_controller_service::{AdmissionLimits, ControllerJournal, WorkerTransportError};
    use sandproof_node::{HostObservation, LinuxWorker, NodeConfig};
    use sandproof_protocol::{WorkAction, WorkReceipt};
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CredentialFile {
        tenant: u64,
        secret: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ConfigFile {
        namespace: String,
        database: PathBuf,
        node_state_dir: PathBuf,
        jailer: PathBuf,
        firecracker: PathBuf,
        chroot_base: PathBuf,
        kernel: PathBuf,
        rootfs: PathBuf,
        uid_base: u32,
        max_vm_id: u64,
        vcpu_count: u8,
        memory_mib: u32,
        file_policy_digest: u64,
        bind: SocketAddr,
        domain: String,
        credentials: Vec<CredentialFile>,
        max_active_global: u64,
        max_active_per_tenant: u64,
        max_disk_bytes_global: u64,
        max_disk_bytes_per_tenant: u64,
        cleanup_reserve_bytes: u64,
        max_timeout_secs: u64,
        default_timeout_secs: u64,
        expiry_scan_seconds: u64,
    }

    pub(crate) struct NodeHost(LinuxWorker);

    impl HostAdapter for NodeHost {
        type Evidence = HostObservation;

        fn available_disk_bytes(&self) -> Result<u64, PortError> {
            self.0.available_disk_bytes().map_err(|_| PortError::Busy)
        }

        fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
            self.0
                .put_secret(sandbox, token)
                .map_err(|_| PortError::Busy)
        }

        fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
            self.0.token_for(sandbox).map_err(|_| PortError::Busy)
        }

        fn execute(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError> {
            self.0.execute(action)
        }
    }

    fn private_root_path(path: &Path) -> Result<(), Box<dyn Error>> {
        sandbox_runtime::check_trusted_path(path)?;
        if !path.is_absolute() || path.is_symlink() {
            return Err("configuration path must be absolute and not a symlink".into());
        }
        let metadata = fs::metadata(path)?;
        if metadata.uid() != 0 || metadata.permissions().mode() & 0o077 != 0 {
            return Err("configuration and database directory must be root-only".into());
        }
        Ok(())
    }

    fn validate_config(config: &ConfigFile) -> Result<u64, Box<dyn Error>> {
        if !config.bind.ip().is_loopback() {
            return Err(
                "HTTP management listener must bind loopback behind a trusted TLS front end".into(),
            );
        }
        if config.credentials.is_empty()
            || config
                .credentials
                .iter()
                .any(|credential| credential.tenant == 0)
        {
            return Err("at least one nonzero tenant credential is required".into());
        }
        if config.expiry_scan_seconds == 0
            || config.default_timeout_secs == 0
            || config.default_timeout_secs > config.max_timeout_secs
        {
            return Err("invalid lifetime scanner or default".into());
        }
        private_root_path(config.database.parent().ok_or("database parent missing")?)?;
        if fs::metadata(&config.node_state_dir)?.dev() != fs::metadata(&config.chroot_base)?.dev() {
            return Err(
                "database and private VM disk must share a filesystem for cleanup reserve".into(),
            );
        }
        let disk_bytes = fs::metadata(&config.rootfs)?.len();
        if disk_bytes == 0 || disk_bytes > config.max_disk_bytes_per_tenant {
            return Err("root disk exceeds per-tenant budget".into());
        }
        Ok(disk_bytes)
    }

    pub async fn run() -> Result<(), Box<dyn Error>> {
        private_root_path(&std::env::current_exe()?)?;
        let mut args = std::env::args_os();
        let _program = args.next();
        let path = PathBuf::from(
            args.next()
                .ok_or("usage: sandproof-server /absolute/root-only/config.json")?,
        );
        if args.next().is_some() {
            return Err("only one configuration path is accepted".into());
        }
        private_root_path(&path)?;
        if !fs::metadata(&path)?.is_file() {
            return Err("configuration must be a regular file".into());
        }
        let config: ConfigFile = serde_json::from_slice(&fs::read(path)?)?;
        let disk_bytes = validate_config(&config)?;
        let node = LinuxWorker::new(NodeConfig {
            namespace: config.namespace.clone(),
            state_dir: config.node_state_dir,
            jailer: config.jailer,
            firecracker: config.firecracker,
            chroot_base: config.chroot_base,
            kernel: config.kernel,
            rootfs: config.rootfs,
            uid_base: config.uid_base,
            max_vm_id: config.max_vm_id,
            vcpu_count: config.vcpu_count,
            memory_mib: config.memory_mib,
            file_policy_digest: core::PolicyDigest(config.file_policy_digest),
        })?;
        let journal = ControllerJournal::open(config.database, &config.namespace)?;
        let port = Arc::new(ManagedPort::new(
            journal,
            NodeHost(node),
            ManagedConfig {
                limits: AdmissionLimits {
                    max_active_global: config.max_active_global,
                    max_active_per_tenant: config.max_active_per_tenant,
                    max_disk_bytes_global: config.max_disk_bytes_global,
                    max_disk_bytes_per_tenant: config.max_disk_bytes_per_tenant,
                    cleanup_reserve_bytes: config.cleanup_reserve_bytes,
                    max_timeout_secs: config.max_timeout_secs,
                },
                disk_bytes,
                file_policy_digest: core::PolicyDigest(config.file_policy_digest),
                default_timeout_secs: config.default_timeout_secs,
                cpu_count: u32::from(config.vcpu_count),
                memory_mb: config.memory_mib,
                disk_size_mb: u32::try_from(disk_bytes.div_ceil(1024 * 1024))?,
                envd_version: "0.9.0".into(),
                domain: config.domain,
            },
        ));
        let credentials = config
            .credentials
            .into_iter()
            .map(|entry| ApiCredential {
                tenant: entry.tenant.to_string(),
                secret: entry.secret,
            })
            .collect();
        let gateway_port = Arc::clone(&port);
        let gateway_namespace = config.namespace.clone();
        let app = router(Arc::clone(&port), credentials)
            .map_err(|_| "invalid API credentials")?
            .fallback(move |request| {
                gateway::handle(
                    Arc::clone(&gateway_port),
                    gateway_namespace.clone(),
                    request,
                )
            });
        let listener = tokio::net::TcpListener::bind(config.bind).await?;
        let scanner = Arc::clone(&port);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(config.expiry_scan_seconds));
            loop {
                timer.tick().await;
                let port = Arc::clone(&scanner);
                match tokio::task::spawn_blocking(move || port.expire_due()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => eprintln!("expiry worker needs reconciliation"),
                    Err(error) => eprintln!("expiry worker join failure: {error}"),
                }
            }
        });
        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "relay") {
        gateway::relay().await
    } else {
        linux::run().await
    }
}
