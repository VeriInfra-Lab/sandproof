#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod gateway;

#[cfg(target_os = "linux")]
mod storage;

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
        time::{Duration, Instant},
    };

    use core::maintenance::slots::HostSlotPhase;
    use sandbox_controller_core as core;
    use sandproof_api::{
        ApiCredential, PortError,
        managed::{HostAdapter, ManagedConfig, ManagedPort, can_install_host_slot_binding},
        router,
    };
    use sandproof_controller_service::{
        AdmissionLimits, BoundedHostConfig, ControllerJournal, CreateOptions, HostSlotContext,
        MaintenanceConfig, WorkerTransportError,
        template_registry::CandidateVerification,
        templates::{TemplateBinding, TemplateCatalog},
    };
    use sandproof_node::{HostObservation, LinuxWorker, NodeConfig, ObservedSlot};
    #[cfg(debug_assertions)]
    use sandproof_protocol::WorkOutcome;
    use sandproof_protocol::{WorkAction, WorkReceipt};
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CredentialFile {
        tenant: u64,
        secret: String,
    }

    /// User settings exclude derived durable limits to avoid two competing
    /// sources of truth for the gateway's physical route-cell capacity.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct BoundedHostSettings {
        #[serde(default)]
        physical_storage:
            Option<sandproof_controller_service::physical_storage::PhysicalStorageProfile>,
        #[serde(default)]
        console_policy: Option<sandproof_controller_service::ConsolePolicy>,
        maintenance: core::maintenance::MaintenanceLimits,
        uid_base: u32,
        uplink_base: u64,
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
        /// Legacy VM-to-UID bound, ignored when `bounded_host` is enabled.
        max_vm_id: u64,
        /// Explicit physical-position maintenance limits; no production defaults.
        #[serde(default)]
        bounded_host: Option<BoundedHostSettings>,
        /// Explicit opt-in for retirement/checkpoints; no inferred limits.
        #[serde(default)]
        maintenance: Option<MaintenanceConfig>,
        /// Explicit private-pool retry-window policy; cannot be disabled once persisted.
        #[serde(default)]
        build_retry_windows: Option<sandproof_controller_service::BuildRetryConfig>,
        vcpu_count: u8,
        memory_mib: u32,
        #[serde(default)]
        host_resources: Option<sandbox_runtime::HostResources>,
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
        #[serde(default)]
        max_lifetime_secs: Option<u64>,
        #[serde(default = "default_startup_timeout_secs")]
        startup_timeout_secs: u64,
        #[serde(default = "default_timeout_secs")]
        default_timeout_secs: u64,
        #[serde(default)]
        public_ipv4_enabled: bool,
        expiry_scan_seconds: u64,
        /// Explicit opt-in while deployment/large-file KVM acceptance is pending.
        #[serde(default)]
        data_transfer: Option<gateway::TransferLimits>,
        /// Explicit opt-in: trusted prebuilt descriptors, not a build service.
        #[serde(default)]
        templates: Option<TemplateCatalog>,
        #[serde(default)]
        template_registry: Option<sandproof_controller_service::template_registry::RegistryConfig>,
        /// Private, separate build pool only; never enable on the runtime endpoint.
        #[serde(default)]
        internal_build_vms: bool,
    }

    fn default_timeout_secs() -> u64 {
        sandproof_controller_service::DEFAULT_TIMEOUT_SECS
    }

    fn default_startup_timeout_secs() -> u64 {
        sandproof_controller_service::DEFAULT_STARTUP_TIMEOUT_SECS
    }

    pub(crate) struct NodeHost(LinuxWorker, Option<crate::storage::PhysicalProfileLease>);

    impl HostAdapter for NodeHost {
        type Evidence = HostObservation;

        fn bind_host_slot(
            &mut self,
            context: &HostSlotContext,
            allow_install: bool,
        ) -> Result<(), PortError> {
            self.0
                .bind_host_slot(context, allow_install)
                .map_err(|_| PortError::Busy)
        }

        fn release_host_slot(&mut self, context: &HostSlotContext) -> Result<(), PortError> {
            self.0
                .release_host_slot(context)
                .map_err(|_| PortError::Busy)
        }

        fn acquire_data_gate(
            &self,
            sandbox: core::SandboxId,
            vm: core::VmId,
            generation: core::Generation,
        ) -> Result<Option<std::fs::File>, PortError> {
            self.0
                .acquire_data_gate(sandbox, vm, generation)
                .map_err(|_| PortError::Busy)
        }

        fn spawn_data_relay(
            &mut self,
            sandbox: core::SandboxId,
            vm: core::VmId,
            generation: core::Generation,
            control: bool,
        ) -> Result<Option<std::process::Child>, PortError> {
            let executable = std::env::current_exe().map_err(|_| PortError::Busy)?;
            self.0
                .spawn_data_relay(sandbox, vm, generation, control, &executable)
                .map_err(|error| {
                    if error
                        .downcast_ref::<sandproof_node::RouteCapacityExceeded>()
                        .is_some()
                    {
                        PortError::QuotaExceeded
                    } else {
                        PortError::Busy
                    }
                })
        }

        fn data_gate_drained(&self, context: &HostSlotContext) -> Result<bool, PortError> {
            self.0
                .data_gate_drained(context)
                .map_err(|_| PortError::Busy)
        }

        fn validate_template(&self, template: &TemplateBinding) -> Result<(), PortError> {
            self.0
                .validate_template(template)
                .map_err(|_| PortError::Busy)
        }

        fn cache_candidate_verification(
            &self,
            template: &TemplateBinding,
            verification: &CandidateVerification,
        ) {
            self.0.cache_candidate_verification(template, verification);
        }

        fn put_bound_launch_config(
            &mut self,
            sandbox: core::SandboxId,
            token: &str,
            options: &CreateOptions,
        ) -> Result<(), PortError> {
            self.0
                .put_bound_launch_secret(sandbox, token, options)
                .map_err(|_| PortError::Busy)
        }

        fn has_bound_launch_config(
            &self,
            sandbox: core::SandboxId,
            options: &CreateOptions,
        ) -> Result<bool, PortError> {
            self.0
                .has_bound_launch_secret(sandbox, options)
                .map_err(|_| PortError::Busy)
        }

        fn public_ipv4_ready(&self) -> Result<bool, PortError> {
            std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
                .map(|value| value.trim() == "1")
                .map_err(|_| PortError::Busy)
        }

        fn available_disk_bytes(&self) -> Result<u64, PortError> {
            self.0.available_disk_bytes().map_err(|_| PortError::Busy)
        }

        fn validate_physical_storage(
            &self,
            profile: &sandproof_controller_service::physical_storage::PhysicalStorageProfile,
            pool: &sandproof_controller_service::physical_storage::PhysicalPool,
        ) -> Result<(), PortError> {
            self.1
                .as_ref()
                .ok_or(PortError::Busy)?
                .validate_admission(profile, pool)
                .map_err(|_| PortError::Busy)
        }

        fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
            self.0
                .put_secret(sandbox, token)
                .map_err(|_| PortError::Busy)
        }

        fn put_launch_config(
            &mut self,
            sandbox: core::SandboxId,
            token: &str,
            env_vars: &std::collections::BTreeMap<String, String>,
        ) -> Result<(), PortError> {
            self.0
                .put_launch_secret(sandbox, token, env_vars)
                .map_err(|_| PortError::Busy)
        }

        fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError> {
            self.0.has_secret(sandbox).map_err(|_| PortError::Busy)
        }

        fn has_launch_config(
            &self,
            sandbox: core::SandboxId,
            env_vars: &std::collections::BTreeMap<String, String>,
        ) -> Result<bool, PortError> {
            self.0
                .has_launch_secret(sandbox, env_vars)
                .map_err(|_| PortError::Busy)
        }

        fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
            self.0.token_for(sandbox).map_err(|_| PortError::Busy)
        }

        fn execute(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError> {
            // Debug-only deterministic crash after the outbox dispatch claim,
            // before any host side effect. Recovery must not assume the
            // action was either executed or safely retryable.
            #[cfg(debug_assertions)]
            {
                let target = std::env::var("SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION").ok();
                if matches!(
                    (target.as_deref(), action.command.command()),
                    (
                        Some("open_fence"),
                        core::Command::InstallFence {
                            terminal: false,
                            ..
                        }
                    ) | (Some("stop_vm"), core::Command::StopVm { .. })
                        | (Some("activate_vm"), core::Command::ActivateVm { .. })
                ) {
                    std::process::exit(87);
                }
            }
            // Test-only, one-shot synchronization point after the durable
            // revocation/fence and before the physical stop. It lets a second
            // HTTP kill arrive while the first owns the lifecycle lock.
            #[cfg(debug_assertions)]
            if matches!(action.command.command(), core::Command::StopVm { .. })
                && let Some(path) = std::env::var_os("SANDPROOF_TEST_STOP_GATE")
            {
                use std::io::Read;
                use std::os::unix::net::UnixListener;
                let path = PathBuf::from(path);
                let parent = path.parent().expect("stop gate must have a parent");
                let metadata = fs::metadata(parent).expect("stop gate parent missing");
                assert_eq!(metadata.uid(), 0, "stop gate parent must be root owned");
                assert_eq!(
                    metadata.mode() & 0o077,
                    0,
                    "stop gate parent must be private"
                );
                let listener = UnixListener::bind(&path).expect("bind stop gate");
                let (mut peer, _) = listener.accept().expect("accept stop gate");
                let mut release = [0_u8; 1];
                peer.read_exact(&mut release)
                    .expect("read stop gate release");
                assert_eq!(release, [b'g']);
                drop(listener);
                fs::remove_file(path).expect("remove stop gate");
            }
            let result = self.0.execute(action);
            // The integration test exits the *whole service* after a real
            // host result but before the journal records its receipt. Release
            // binaries have no fault injection path.
            #[cfg(debug_assertions)]
            if result
                .as_ref()
                .is_ok_and(|receipt| matches!(receipt.outcome, WorkOutcome::ObservedSuccess(_)))
            {
                let target = std::env::var("SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION").ok();
                let command = action.command.command();
                if matches!(
                    (target.as_deref(), command),
                    (Some("prepare_vm"), core::Command::PrepareVm { .. })
                        | (
                            Some("configure_network"),
                            core::Command::ConfigureNetwork { .. }
                        )
                        | (Some("stop_vm"), core::Command::StopVm { .. })
                        | (Some("activate_vm"), core::Command::ActivateVm { .. })
                        | (Some("abort_prepare"), core::Command::AbortPrepare { .. })
                        | (Some("cleanup_vm"), core::Command::CleanupVm { .. })
                ) {
                    std::process::exit(86);
                }
            }
            result
        }

        fn observe(
            &self,
            action: &WorkAction<core::EffectId>,
        ) -> Option<WorkReceipt<Self::Evidence>> {
            match self.0.observe_action(action) {
                Ok(receipt) => Some(receipt),
                Err(error) => {
                    let kind = match action.command.command() {
                        core::Command::InstallFence {
                            terminal: false, ..
                        } => "open fence",
                        core::Command::InstallFence { terminal: true, .. } => "terminal fence",
                        core::Command::PrepareVm { .. } => "prepare VM",
                        core::Command::ActivateVm { .. } => "activate VM",
                        core::Command::ConfigureNetwork { .. } => "configure network",
                        core::Command::StopVm { .. } => "stop VM",
                        core::Command::AbortPrepare { .. } => "abort prepare",
                        core::Command::CleanupVm { .. } => "cleanup VM",
                        _ => "other host action",
                    };
                    eprintln!("host {kind} observation remains unknown: {error}");
                    None
                }
            }
        }

        fn reclaim_incomplete_prepare(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            #[cfg(debug_assertions)]
            if std::env::var("SANDPROOF_TEST_FAIL_RECLAIM_PREPARE_SANDBOX")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                == Some(action.command.command().sandbox().0)
            {
                // Refuse only this sandbox's compensating observation. The
                // original PrepareVm remains unknown; no launch is replayed.
                return Err(PortError::Busy);
            }
            self.0
                .reclaim_incomplete_prepare(action)
                .map_err(|_| PortError::Busy)
        }

        fn reclaim_incomplete_network(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            self.0
                .reclaim_incomplete_network(action)
                .map_err(|_| PortError::Busy)
        }

        fn reclaim_incomplete_activation(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            self.0
                .reclaim_incomplete_activation(action)
                .map_err(|_| PortError::Busy)
        }

        fn reclaim_incomplete_terminal_fence(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            self.0
                .reclaim_incomplete_terminal_fence(action)
                .map_err(|_| PortError::Busy)
        }

        fn reclaim_incomplete_open_fence(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            self.0
                .reclaim_incomplete_open_fence(action)
                .map_err(|_| PortError::Busy)
        }

        fn reclaim_incomplete_stop(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            #[cfg(debug_assertions)]
            if std::env::var("SANDPROOF_TEST_FAIL_RECLAIM_STOP_SANDBOX")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                == Some(action.command.command().sandbox().0)
            {
                // Test-only refusal before host I/O. The old stop remains
                // unknown, so another sandbox must still be able to recover.
                return Err(PortError::Busy);
            }
            self.0
                .reclaim_incomplete_stop(action)
                .map_err(|_| PortError::Busy)
        }

        fn reclaim_incomplete_abort_prepare(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            self.0
                .reclaim_incomplete_abort_prepare(action)
                .map_err(|_| PortError::Busy)
        }

        fn reclaim_incomplete_cleanup(
            &mut self,
            action: &WorkAction<core::EffectId>,
        ) -> Result<(), PortError> {
            self.0
                .reclaim_incomplete_cleanup(action)
                .map_err(|_| PortError::Busy)
        }
    }

    fn validate_physical_storage(
        config: &ConfigFile,
        node: &NodeConfig,
    ) -> Result<Option<crate::storage::PhysicalProfileLease>, Box<dyn Error>> {
        use sandproof_controller_service::physical_storage::PoolRole;
        let profile = node
            .bounded_host
            .as_ref()
            .and_then(|value| value.physical_storage.as_ref());
        let capacity = config
            .maintenance
            .as_ref()
            .and_then(|value| value.capacity.as_ref());
        if profile.is_none() && capacity.is_none() {
            return Ok(None);
        }
        let profile =
            profile.ok_or("full bounded capacity requires an explicit physical_storage profile")?;
        let capacity = capacity.ok_or("physical_storage requires full controller capacity")?;
        let pool = profile.pool(&config.namespace)?;
        let expected_role = if config.internal_build_vms {
            PoolRole::Builder
        } else {
            PoolRole::Runtime
        };
        let expected_kernel = config
            .templates
            .as_ref()
            .map_or(0, |catalog| {
                catalog
                    .templates
                    .iter()
                    .map(|template| template.kernel.size_bytes)
                    .max()
                    .unwrap_or(0)
            })
            .max(fs::metadata(&config.kernel)?.len())
            .max(
                config
                    .template_registry
                    .as_ref()
                    .map_or(0, |registry| registry.max_artifact_bytes_per_tenant),
            );
        let envelope =
            sandproof_node::HostMetadataEnvelope::derive(node, capacity.max_live_sandboxes)?;
        if pool.role != expected_role
            || pool.database != config.database
            || pool.node_state_dir != config.node_state_dir
            || pool.chroot_base != config.chroot_base
            || node
                .bounded_host
                .as_ref()
                .map(|host| host.maintenance.max_host_slots)
                != Some(pool.max_host_slots)
            || config
                .max_disk_bytes_global
                .checked_sub(config.cleanup_reserve_bytes)
                != Some(pool.rootfs_budget_bytes)
            || pool.cleanup_reserve_bytes != config.cleanup_reserve_bytes
            || pool.kernel_copy_bytes < expected_kernel
            || pool.runtime_copy_bytes < fs::metadata(&config.firecracker)?.len()
            || pool.host_metadata_bytes < envelope.payload_bytes
            || pool.host_metadata_inodes < envelope.inode_count
            || pool
                .host_metadata_bytes
                .checked_add(capacity.max_online_storage_bytes)
                .is_none_or(|bytes| bytes > pool.cleanup_reserve_bytes)
        {
            return Err("physical profile does not cover this exact pool configuration".into());
        }
        match (&config.template_registry, &profile.public_build) {
            (Some(registry), Some(public)) if !config.internal_build_vms => {
                profile.validate_registry(&config.namespace, registry)?;
                if public.registry_root != registry.root {
                    return Err("physical profile registry root mismatch".into());
                }
            }
            (None, None) if !config.internal_build_vms => {}
            (None, Some(_)) if config.internal_build_vms => {}
            _ => return Err("unsupported physical profile registry/pool composition".into()),
        }
        let space = sandproof_node::disk_space(&profile.coordination_file)?;
        if profile
            .filesystem_overhead_inodes
            .checked_mul(space.fragment_bytes)
            .is_none_or(|bytes| bytes > profile.filesystem_overhead_bytes)
        {
            return Err("physical filesystem overhead must cover per-inode block rounding".into());
        }
        Ok(Some(crate::storage::PhysicalProfileLease::activate(
            profile,
            &config.namespace,
            &config.firecracker,
            &config.kernel,
        )?))
    }

    fn node_configuration(
        config: &ConfigFile,
        bounded_host: Option<BoundedHostConfig>,
    ) -> NodeConfig {
        NodeConfig {
            namespace: config.namespace.clone(),
            state_dir: config.node_state_dir.clone(),
            jailer: config.jailer.clone(),
            firecracker: config.firecracker.clone(),
            chroot_base: config.chroot_base.clone(),
            kernel: config.kernel.clone(),
            rootfs: config.rootfs.clone(),
            uid_base: config.uid_base,
            max_vm_id: config.max_vm_id,
            bounded_host,
            vcpu_count: config.vcpu_count,
            memory_mib: config.memory_mib,
            host_resources: config.host_resources.clone(),
            file_policy_digest: core::PolicyDigest(config.file_policy_digest),
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
        validate_controller_storage_reserve(config)?;
        resolve_bounded_host_config(
            config.uid_base,
            config.bounded_host.as_ref(),
            config.data_transfer.as_ref(),
        )?;
        if let Some(registry) = &config.template_registry {
            if !registry.is_valid() || config.templates.is_none() || config.host_resources.is_none()
            {
                return Err(
                    "template registry requires valid limits, templates and host_resources".into(),
                );
            }
            for path in [
                &registry.root,
                &registry.root.join("inbox"),
                &registry.root.join("artifacts"),
            ] {
                private_root_path(path)?;
                if !fs::metadata(path)?.is_dir() {
                    return Err("registry path is not a directory".into());
                }
            }
            crate::storage::validate_filesystems(
                &config.database,
                &registry.root,
                &config.chroot_base,
            )?;
        }
        if let Some(catalog) = &config.templates {
            catalog
                .validate()
                .map_err(|_| "invalid prebuilt template catalog")?;
            if config.host_resources.is_none() {
                return Err("templates require explicit host_resources".into());
            }
        }
        if let Some(limits) = &config.data_transfer {
            limits.validate()?;
        }
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
            || config.startup_timeout_secs == 0
            || config.default_timeout_secs != default_timeout_secs()
            || config.default_timeout_secs > config.max_timeout_secs
            || config.max_lifetime_secs == Some(0)
        {
            return Err("invalid lifetime scanner/limit; E2B default must be 300 seconds".into());
        }
        private_root_path(config.database.parent().ok_or("database parent missing")?)?;
        crate::storage::validate_filesystems(
            &config.database,
            &config.node_state_dir,
            &config.chroot_base,
        )?;
        let disk_bytes = fs::metadata(&config.rootfs)?.len();
        if disk_bytes == 0 || disk_bytes > config.max_disk_bytes_per_tenant {
            return Err("root disk exceeds per-tenant budget".into());
        }
        Ok(disk_bytes)
    }

    fn resolve_bounded_host_config(
        uid_base: u32,
        bounded: Option<&BoundedHostSettings>,
        data_transfer: Option<&gateway::TransferLimits>,
    ) -> Result<Option<BoundedHostConfig>, Box<dyn Error>> {
        let Some(settings) = bounded else {
            return Ok(None);
        };
        let transfers =
            data_transfer.ok_or("bounded host requires explicit data_transfer limits")?;
        transfers.validate()?;
        let bounded = BoundedHostConfig {
            physical_storage: settings.physical_storage.clone(),
            console_policy: settings.console_policy,
            maintenance: settings.maintenance.clone(),
            uid_base: settings.uid_base,
            uplink_base: settings.uplink_base,
            max_data_routes_per_slot: u64::try_from(transfers.max_transfers_per_vm)?,
            max_data_routes_global: u64::try_from(transfers.max_transfers)?,
            data_route_control_reserve: u64::try_from(transfers.control_reserve)?,
        };
        bounded.validate()?;
        if bounded.uid_base != uid_base {
            return Err("bounded host UID base differs from node UID base".into());
        }
        Ok(Some(bounded))
    }

    fn validate_maintenance_config(
        bounded: Option<&BoundedHostConfig>,
        maintenance: Option<&MaintenanceConfig>,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(maintenance) = maintenance {
            if bounded.is_none() {
                return Err("maintenance requires explicit bounded_host configuration".into());
            }
            maintenance.validate()?;
            if maintenance.capacity.is_none() {
                return Err(
                    "production maintenance requires explicit cleanup capacity reservations".into(),
                );
            }
            if maintenance.capacity.is_some()
                && bounded.and_then(|config| config.console_policy)
                    != Some(sandproof_controller_service::ConsolePolicy::Discard)
            {
                return Err(
                    "full bounded capacity requires explicit bounded_host.console_policy=discard"
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn validate_controller_storage_reserve(config: &ConfigFile) -> Result<(), Box<dyn Error>> {
        if let Some(capacity) = config
            .maintenance
            .as_ref()
            .and_then(|policy| policy.capacity.as_ref())
            && config.cleanup_reserve_bytes < capacity.max_online_storage_bytes
        {
            return Err("cleanup_reserve_bytes must cover the full controller DB/WAL/SHM/checkpoint envelope".into());
        }
        Ok(())
    }

    fn validate_build_retry_config(
        config: &ConfigFile,
        bounded: Option<&BoundedHostConfig>,
    ) -> Result<(), Box<dyn Error>> {
        if config.internal_build_vms
            && config
                .maintenance
                .as_ref()
                .is_some_and(|policy| policy.capacity.is_some())
            && config.build_retry_windows.is_none()
        {
            return Err("full private capacity requires explicit build_retry_windows".into());
        }
        if let Some(retry) = config.build_retry_windows {
            if !config.internal_build_vms
                || config.maintenance.is_none()
                || config.template_registry.is_some()
            {
                return Err(
                    "build_retry_windows requires an explicitly maintained private build pool"
                        .into(),
                );
            }
            let bounded = bounded.ok_or("build_retry_windows requires bounded_host")?;
            retry.validate(bounded.maintenance.windows.max_duration_ms())?;
        }
        Ok(())
    }

    fn has_current_uncertain_prepare(
        journal: &ControllerJournal,
        world: &core::World,
        slot: &ObservedSlot,
        uncertain: &[WorkAction<core::EffectId>],
    ) -> Result<bool, Box<dyn Error>> {
        for action in uncertain {
            if let core::Command::PrepareVm {
                sandbox,
                generation,
                vm,
                slot: network_slot,
                ..
            } = action.command.command()
                && *sandbox == slot.sandbox
                && *generation == slot.generation
                && *vm == slot.vm
                && *network_slot == slot.slot
                && journal.matches_current_prepare(world, action)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn audit_host_inventory(
        node: &LinuxWorker,
        journal: &ControllerJournal,
    ) -> Result<(), Box<dyn Error>> {
        let host_slots = node.host_slot_inventory()?;
        let durable_slots = journal.host_slot_inventory()?;
        let world = journal.world()?;
        audit_host_slot_contexts(&durable_slots, &host_slots, |context| {
            can_install_host_slot_binding(&world, context.binding.sandbox)
        })?;
        let inventory = node.inventory()?;
        if !inventory.unbound_cgroups.is_empty() {
            return Err(format!(
                "unbound VM cgroups require operator reconciliation: {:?}",
                inventory.unbound_cgroups
            )
            .into());
        }
        if !inventory.unbound_claims.is_empty() {
            return Err(format!(
                "unbound VM claims require operator reconciliation: {:?}",
                inventory.unbound_claims
            )
            .into());
        }
        if !inventory.unbound_networks.is_empty() {
            return Err(format!(
                "unbound network namespaces require operator reconciliation: {:?}",
                inventory.unbound_networks
            )
            .into());
        }
        let uncertain = journal.uncertain_actions()?;
        for slot in inventory.slots {
            if world.maintenance_state().is_some()
                && !host_slots.iter().any(|(context, phase)| {
                    *phase == HostSlotPhase::Occupied
                        && context.binding.sandbox == slot.sandbox
                        && context.binding.generation == slot.generation
                        && context.binding.vm == slot.vm
                        && context.binding.network == slot.slot
                        && slot.owner.as_str() == format!("tenant-{}", context.binding.owner.0)
                })
            {
                return Err("host resource has no matching occupied physical position".into());
            }
            audit_host_slot(node, journal, &world, &uncertain, &slot)?;
        }
        Ok(())
    }

    /// Compare durable identities before inspecting live resources. Only the
    /// two transaction boundaries may lag: journal bind before node bind, and
    /// journal `BeginRelease` before node release/`ConfirmRelease`. Free tombstones
    /// remain mandatory, but never require a retired sandbox's old secret.
    fn audit_host_slot_contexts(
        durable: &[(HostSlotContext, HostSlotPhase)],
        observed: &[(HostSlotContext, HostSlotPhase)],
        can_install: impl Fn(&HostSlotContext) -> bool,
    ) -> Result<(), Box<dyn Error>> {
        for records in [durable, observed] {
            let mut positions = std::collections::BTreeSet::new();
            let mut identities = std::collections::BTreeSet::new();
            if records.iter().any(|(context, _)| {
                !positions.insert(context.position.0) || !identities.insert(context.binding.vm.0)
            }) {
                return Err("duplicate bounded host position or VM identity".into());
            }
        }
        let observed_by_position: std::collections::BTreeMap<_, _> = observed
            .iter()
            .map(|record| (record.0.position.0, record))
            .collect();
        let durable_positions: std::collections::BTreeSet<_> =
            durable.iter().map(|record| record.0.position.0).collect();
        for (context, phase) in durable {
            let host = observed_by_position.get(&context.position.0).copied();
            let valid = match (phase, host) {
                (HostSlotPhase::Occupied, None) => {
                    context.expected_previous.is_none() && can_install(context)
                }
                (HostSlotPhase::Occupied, Some((prior, HostSlotPhase::Free))) => {
                    context.expected_previous == Some(prior.binding.vm)
                        && context.binding.vm.0 > prior.binding.vm.0
                        && can_install(context)
                }
                (HostSlotPhase::Occupied, Some((current, HostSlotPhase::Occupied)))
                | (HostSlotPhase::Releasing, Some((current, _)))
                | (HostSlotPhase::Free, Some((current, HostSlotPhase::Free))) => context == current,
                _ => false,
            };
            if !valid {
                return Err("bounded host binding differs from durable controller position".into());
            }
        }
        if observed
            .iter()
            .any(|(context, _)| !durable_positions.contains(&context.position.0))
        {
            return Err("bounded host position has no durable controller binding".into());
        }
        Ok(())
    }

    fn audit_host_slot(
        node: &LinuxWorker,
        journal: &ControllerJournal,
        world: &core::World,
        uncertain: &[WorkAction<core::EffectId>],
        slot: &ObservedSlot,
    ) -> Result<(), Box<dyn Error>> {
        if !node.has_bound_launch_secret(slot.sandbox, &journal.create_options(slot.sandbox)?)? {
            return Err("host slot has no matching durable launch configuration".into());
        }
        let record = world
            .sandbox_record(slot.sandbox)
            .ok_or("host slot has no durable sandbox")?;
        if slot.owner.as_str() != format!("tenant-{}", record.owner.0) {
            return Err("host slot owner differs from durable sandbox".into());
        }
        let planned = world.operation_records().iter().any(|(key, _)| {
            key.sandbox == slot.sandbox
                && world.operation_plan(*key).is_some_and(|plan| {
                    plan.generation == slot.generation
                        && plan.vm == Some(slot.vm)
                        && plan.slot == Some(slot.slot)
                })
        });
        if !planned {
            return Err("host slot has no matching durable resource plan".into());
        }
        if slot.helper_pending {
            // The exact occupied incarnation has a known live helper. Keep
            // its outcome unknown locally; other positions may recover and
            // serve queries. Identity/config corruption already failed above.
            return Ok(());
        }
        if (slot.aborted_unlaunched || slot.failed_prepare_reclaimed)
            && ((slot.aborted_unlaunched && slot.process.is_some())
                || slot.network_inode.is_some()
                || slot.disk_inode.is_some()
                || slot.cgroup_inode.is_some())
        {
            return Err("reclaimed failed PrepareVm still has physical resources".into());
        }
        if slot.claim_present && slot.process.is_none() && !slot.aborted_unlaunched {
            // A claim from the exact current uncertain PrepareVm may be
            // an interrupted pre-launch. The per-sandbox recovery path
            // will reclaim it or park this sandbox. All other claims
            // remain a global startup error, never an inferred stop.
            if record.phase == core::Phase::Destroyed
                || !has_current_uncertain_prepare(journal, world, slot, uncertain)?
            {
                return Err(
                    "VM claim has no exact process record; manual reconciliation required".into(),
                );
            }
        }
        if let Some(vm) = world.vm_record(slot.vm) {
            if vm.sandbox != slot.sandbox || vm.generation != slot.generation {
                return Err("host VM identity differs from durable resource".into());
            }
            if vm.status == core::VmStatus::Reclaimed
                && (slot.disk_inode.is_some()
                    || slot.network_inode.is_some()
                    || slot.cgroup_inode.is_some()
                    || slot
                        .process
                        .as_ref()
                        .is_some_and(|process| process.state != sandbox_runtime::VmState::Stopped))
            {
                return Err("reclaimed VM still has physical resources".into());
            }
        }
        if let Some(network) = world.slot_record(slot.slot) {
            if network.sandbox != slot.sandbox
                || network.generation != slot.generation
                || network.vm != slot.vm
            {
                return Err("host network identity differs from durable resource".into());
            }
            if network.reclaimed && slot.network_inode.is_some() {
                return Err("reclaimed network slot still exists on host".into());
            }
        }
        if record.phase == core::Phase::Destroyed
            && slot
                .process
                .as_ref()
                .is_some_and(|process| process.state != sandbox_runtime::VmState::Stopped)
        {
            return Err("terminal sandbox still has a live or unobservable VM".into());
        }
        Ok(())
    }

    fn spawn_reconciliation_scanner(port: Arc<ManagedPort<NodeHost>>, seconds: u64) {
        drop(tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(seconds));
            let mut last_failure_log: Option<Instant> = None;
            loop {
                timer.tick().await;
                let port = Arc::clone(&port);
                match tokio::task::spawn_blocking(move || {
                    // Free tail capacity before recovery/expiry append inputs.
                    port.maintain()?;
                    let recovery = port.recover_on_startup()?;
                    let expiry = port.expire_due_report()?;
                    let maintenance = port.maintain()?;
                    Ok::<_, PortError>((recovery, expiry, maintenance))
                })
                .await
                {
                    Ok(Ok((recovery, expiry, maintenance)))
                        if recovery.unresolved_sandboxes > 0
                            || expiry.unresolved_sandboxes > 0
                            || maintenance.pending_sandboxes > 0 =>
                    {
                        if last_failure_log
                            .is_none_or(|instant| instant.elapsed() >= Duration::from_mins(1))
                        {
                            eprintln!(
                                "reconciliation left {} sandboxes or physical-slot releases unresolved ({} after host or receipt errors); {} due expiries and {} metadata retirements remain unresolved; manual inspection may be required",
                                recovery.unresolved_sandboxes,
                                recovery.failed_sandboxes,
                                expiry.unresolved_sandboxes,
                                maintenance.pending_sandboxes
                            );
                            last_failure_log = Some(Instant::now());
                        }
                    }
                    Ok(Ok(_)) => last_failure_log = None,
                    Ok(Err(_)) => {
                        eprintln!("reconciliation, expiry or maintenance worker needs attention");
                    }
                    Err(error) => eprintln!("reconciliation worker join failure: {error}"),
                }
            }
        }));
    }

    fn managed_config(
        config: &ConfigFile,
        disk_bytes: u64,
    ) -> Result<ManagedConfig, Box<dyn Error>> {
        Ok(ManagedConfig {
            public_ipv4_enabled: config.public_ipv4_enabled,
            limits: AdmissionLimits {
                max_vm_id: config.max_vm_id,
                max_active_global: config.max_active_global,
                max_active_per_tenant: config.max_active_per_tenant,
                max_disk_bytes_global: config.max_disk_bytes_global,
                max_disk_bytes_per_tenant: config.max_disk_bytes_per_tenant,
                cleanup_reserve_bytes: config.cleanup_reserve_bytes,
                max_timeout_secs: config.max_timeout_secs,
                max_lifetime_secs: config.max_lifetime_secs.unwrap_or(config.max_timeout_secs),
                startup_timeout_secs: config.startup_timeout_secs,
            },
            disk_bytes,
            file_policy_digest: core::PolicyDigest(config.file_policy_digest),
            cpu_count: u32::from(config.vcpu_count),
            memory_mb: config.memory_mib,
            disk_size_mb: u32::try_from(disk_bytes.div_ceil(1024 * 1024))?,
            envd_version: "0.9.0".into(),
            domain: config.domain.clone(),
        })
    }

    fn open_journal(
        database: PathBuf,
        namespace: &str,
        bounded: Option<BoundedHostConfig>,
        maintenance: Option<MaintenanceConfig>,
    ) -> Result<ControllerJournal, Box<dyn Error>> {
        validate_maintenance_config(bounded.as_ref(), maintenance.as_ref())?;
        // Node construction has already independently refused legacy metadata
        // or resources. The journal now rechecks its own empty state/config in
        // the enable transaction; neither check substitutes for the other.
        Ok(if let Some(bounded) = bounded {
            let now_ms = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis(),
            )?;
            if let Some(maintenance) = maintenance {
                ControllerJournal::open_maintained(
                    database,
                    namespace,
                    bounded,
                    maintenance,
                    now_ms,
                )?
            } else {
                ControllerJournal::open_bounded(database, namespace, bounded, now_ms)?
            }
        } else {
            if maintenance.is_some() {
                return Err("maintenance requires explicit bounded_host configuration".into());
            }
            ControllerJournal::open(database, namespace)?
        })
    }

    fn read_configuration() -> Result<ConfigFile, Box<dyn Error>> {
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
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    fn recover_before_serving(port: &ManagedPort<NodeHost>) -> Result<(), Box<dyn Error>> {
        port.maintain().map_err(|error| {
            std::io::Error::other(format!("startup maintenance failed: {error:?}"))
        })?;
        let recovery = port.recover_on_startup().map_err(|error| {
            std::io::Error::other(format!("startup recovery failed: {error:?}"))
        })?;
        if recovery.unresolved_sandboxes != 0 {
            eprintln!(
                "startup recovery left {} sandboxes unresolved ({} after host or receipt errors); no uncertain host action was repeated",
                recovery.unresolved_sandboxes, recovery.failed_sandboxes
            );
        }
        Ok(())
    }

    fn preflight_journal_configuration(
        config: &ConfigFile,
        bounded_host: Option<&BoundedHostConfig>,
    ) -> Result<(), Box<dyn Error>> {
        ControllerJournal::preflight_build_retry_windows(
            &config.database,
            &config.namespace,
            config.build_retry_windows,
        )?;
        if let Some(bounded) = bounded_host {
            if let Some(maintenance) = &config.maintenance {
                ControllerJournal::preflight_maintained(
                    &config.database,
                    &config.namespace,
                    bounded,
                    maintenance,
                )?;
            } else {
                ControllerJournal::preflight_bounded(&config.database, &config.namespace, bounded)?;
            }
        }
        Ok(())
    }

    pub async fn run() -> Result<(), Box<dyn Error>> {
        let config = read_configuration()?;
        let disk_bytes = validate_config(&config)?;
        let managed = managed_config(&config, disk_bytes)?;
        let bounded_host = resolve_bounded_host_config(
            config.uid_base,
            config.bounded_host.as_ref(),
            config.data_transfer.as_ref(),
        )?;
        validate_maintenance_config(bounded_host.as_ref(), config.maintenance.as_ref())?;
        validate_build_retry_config(&config, bounded_host.as_ref())?;
        preflight_journal_configuration(&config, bounded_host.as_ref())?;
        let node_config = node_configuration(&config, bounded_host.clone());
        let physical_storage = validate_physical_storage(&config, &node_config)?;
        let node = LinuxWorker::new(node_config)?;
        let journal = open_journal(
            config.database,
            &config.namespace,
            bounded_host,
            config.maintenance,
        )?;
        if journal
            .build_retry_config()?
            .is_some_and(|stored| Some(stored) != config.build_retry_windows)
        {
            return Err(
                "durable private build retry configuration cannot be disabled or changed".into(),
            );
        }
        if journal.has_template_registry()? && config.template_registry.is_none() {
            return Err("durable template registry cannot be disabled for this journal".into());
        }
        audit_host_inventory(&node, &journal)?;
        let port = if let Some(catalog) = config.templates {
            ManagedPort::with_templates(journal, NodeHost(node, physical_storage), managed, catalog)
                .map_err(|_| "invalid prebuilt template configuration")?
        } else {
            ManagedPort::new(journal, NodeHost(node, physical_storage), managed)
        };
        if config.internal_build_vms && config.template_registry.is_some() {
            return Err("internal build pool cannot also publish templates".into());
        }
        let port = port.with_internal_build_vms(config.internal_build_vms);
        let port = if let Some(retry) = config.build_retry_windows {
            port.with_build_retry_windows(retry)
                .map_err(|_| "invalid durable private build retry policy")?
        } else {
            port
        };
        let port = Arc::new(if let Some(registry) = config.template_registry {
            port.with_registry(
                sandproof_controller_service::template_registry::CandidateStore::new(registry, 0)?,
            )
            .map_err(|_| "invalid durable template registry configuration")?
        } else {
            port
        });
        recover_before_serving(&port)?;
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
        let gateway_limits = config
            .data_transfer
            .map(gateway::Gateway::new)
            .transpose()?
            .map(Arc::new);
        let write_idle = gateway_limits
            .as_ref()
            .map(|gateway| Duration::from_secs(gateway.limits.write_idle_seconds));
        let app = router(Arc::clone(&port), credentials)
            .map_err(|_| "invalid API credentials")?
            .fallback(move |request| {
                gateway::handle(
                    Arc::clone(&gateway_port),
                    gateway_namespace.clone(),
                    gateway_limits.clone(),
                    request,
                )
            });
        let listener = tokio::net::TcpListener::bind(config.bind).await?;
        spawn_reconciliation_scanner(Arc::clone(&port), config.expiry_scan_seconds);
        axum::serve(gateway::GatewayListener::new(listener, write_idle), app).await?;
        Ok(())
    }

    #[cfg(test)]
    mod bounded_host_tests {
        use super::*;
        use sandbox_controller_core::maintenance::slots::{HostBinding, HostSlotId};
        use serde_json::{Value, json};

        fn settings() -> Value {
            json!({
                "namespace": "0123456789abcdef", "database": "/state/controller.db",
                "node_state_dir": "/state/node", "jailer": "/bin/jailer",
                "firecracker": "/bin/firecracker", "chroot_base": "/state/jails",
                "kernel": "/images/kernel", "rootfs": "/images/rootfs",
                "uid_base": 1000, "max_vm_id": u64::MAX, "vcpu_count": 1,
                "memory_mib": 128, "file_policy_digest": 1, "bind": "127.0.0.1:3000",
                "domain": "localhost", "credentials": [{"tenant": 1, "secret": "test-only"}],
                "max_active_global": 2, "max_active_per_tenant": 2,
                "max_disk_bytes_global": 4096, "max_disk_bytes_per_tenant": 4096,
                "cleanup_reserve_bytes": 1024, "max_timeout_secs": 300,
                "expiry_scan_seconds": 1,
                "data_transfer": {
                    "file_upload_bytes": 1024, "file_download_bytes": 1024,
                    "command_input_bytes": 1024, "command_output_bytes": 1024,
                    "rpc_request_bytes": 1024, "rpc_response_bytes": 1024,
                    "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
                    "request_idle_seconds": 5, "file_idle_seconds": 5, "write_idle_seconds": 5
                },
                "bounded_host": {"uid_base": 1000, "uplink_base": 0, "maintenance": {
                    "max_retained_results": 10, "max_host_slots": 2,
                    "windows": {"max_open_windows": 2, "max_duration_ms": 100}
                }}
            })
        }

        fn check(value: Value) -> Result<(), Box<dyn Error>> {
            let config: ConfigFile = serde_json::from_value(value)?;
            let bounded = resolve_bounded_host_config(
                config.uid_base,
                config.bounded_host.as_ref(),
                config.data_transfer.as_ref(),
            )?;
            validate_maintenance_config(bounded.as_ref(), config.maintenance.as_ref())?;
            validate_build_retry_config(&config, bounded.as_ref())?;
            validate_controller_storage_reserve(&config)?;
            Ok(())
        }

        fn resolve(value: Value) -> BoundedHostConfig {
            let config: ConfigFile = serde_json::from_value(value).unwrap();
            resolve_bounded_host_config(
                config.uid_base,
                config.bounded_host.as_ref(),
                config.data_transfer.as_ref(),
            )
            .unwrap()
            .unwrap()
        }

        fn maintained_settings() -> Value {
            let mut value = settings();
            value["maintenance"] = json!({
                "retention_ms": 60_000, "max_checkpoint_bytes": 8_000_000,
                "max_records": 10_000, "max_references": 100_000,
                "max_tail_inputs": 10_000, "max_tail_bytes": 8_000_000,
                "max_input_bytes": 128_000, "prune_batch_rows": 3
            });
            value
        }

        fn full_capacity_settings() -> Value {
            // Deliberately small fixture workload; these are not deployment defaults.
            let mut value = maintained_settings();
            value["maintenance"] = json!({
                "retention_ms": 100, "max_checkpoint_bytes": 4 * 1024 * 1024,
                "max_records": 10_000, "max_references": 1_000_000,
                "max_tail_inputs": 8, "max_tail_bytes": 64 * 1024,
                "max_input_bytes": 8192, "prune_batch_rows": 3,
                "capacity": {
                    "max_live_sandboxes": 4, "max_sql_rows": 1024,
                    "max_sql_bytes": 16 * 1024 * 1024, "max_sql_value_bytes": 256 * 1024,
                    "max_receipt_bytes": 16 * 1024, "max_transaction_inputs": 4,
                    "max_transaction_bytes": 32 * 1024,
                    "max_database_bytes": 512 * 1024 * 1024,
                    "max_wal_bytes": 520 * 1024 * 1024, "max_shm_bytes": 2 * 1024 * 1024,
                    "checkpoint_scratch_bytes": 8 * 1024 * 1024,
                    "max_online_storage_bytes": 1042_u64 * 1024 * 1024
                }
            });
            value["bounded_host"]["console_policy"] = json!("discard");
            value["cleanup_reserve_bytes"] = json!(1042_u64 * 1024 * 1024);
            value
        }

        #[test]
        fn full_capacity_requires_explicit_console_and_linked_controller_reserve() {
            let value = full_capacity_settings();
            assert!(check(value.clone()).is_ok());
            let mut missing = value.clone();
            missing["bounded_host"]
                .as_object_mut()
                .unwrap()
                .remove("console_policy");
            assert!(check(missing).is_err());
            let mut short = value.clone();
            short["cleanup_reserve_bytes"] = json!(1042_u64 * 1024 * 1024 - 1);
            assert!(check(short).is_err());
            for field in [
                "max_shm_bytes",
                "max_receipt_bytes",
                "max_transaction_inputs",
            ] {
                let mut missing = value.clone();
                missing["maintenance"]["capacity"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert!(check(missing).is_err());
                let mut zero = value.clone();
                zero["maintenance"]["capacity"][field] = json!(0);
                assert!(check(zero).is_err());
            }
        }

        #[test]
        fn production_maintenance_cannot_accept_work_without_cleanup_reservations() {
            let value = maintained_settings();
            assert!(check(value.clone()).is_err());
            let config: ConfigFile = serde_json::from_value(value).unwrap();
            let database = std::env::temp_dir().join(format!(
                "sandproof-unreserved-maintenance-{}.db",
                std::process::id()
            ));
            assert!(
                open_journal(
                    database.clone(),
                    &config.namespace,
                    Some(resolve(settings())),
                    config.maintenance
                )
                .is_err()
            );
            // Refusal must happen before any database/host initialization.
            assert!(!database.exists());
        }

        #[test]
        fn explicit_console_policy_is_strict_and_not_inferred() {
            assert!(resolve(settings()).console_policy.is_none());
            let mut value = settings();
            value["bounded_host"]["console_policy"] = json!("discard");
            assert_eq!(
                resolve(value.clone()).console_policy,
                Some(sandproof_controller_service::ConsolePolicy::Discard)
            );
            value["bounded_host"]["console_policy"] = json!("file");
            assert!(check(value).is_err());
        }

        #[test]
        fn full_private_capacity_never_falls_back_to_legacy_retry_anchors() {
            let mut value = full_capacity_settings();
            value["internal_build_vms"] = json!(true);
            assert!(check(value.clone()).is_err());
            value["build_retry_windows"] = json!({"window_duration_ms": 100, "max_anchors": 2});
            assert!(check(value).is_ok());
        }

        #[test]
        fn private_retry_policy_requires_explicit_maintained_private_pool() {
            let mut value = full_capacity_settings();
            value["internal_build_vms"] = json!(true);
            value["build_retry_windows"] = json!({"window_duration_ms": 100, "max_anchors": 2});
            assert!(check(value.clone()).is_ok());
            for field in ["internal_build_vms", "maintenance", "bounded_host"] {
                let mut invalid = value.clone();
                invalid.as_object_mut().unwrap().remove(field);
                assert!(check(invalid).is_err(), "missing {field}");
            }
            for field in ["window_duration_ms", "max_anchors"] {
                let mut invalid = value.clone();
                invalid["build_retry_windows"][field] = json!(0);
                assert!(check(invalid).is_err());
                invalid = value.clone();
                invalid["build_retry_windows"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert!(check(invalid).is_err());
            }
            value["build_retry_windows"]["unknown"] = json!(1);
            assert!(check(value).is_err());
        }

        #[test]
        fn maintenance_configuration_is_explicit_strict_and_requires_bounded_host() {
            assert!(check(settings()).is_ok());
            assert!(check(full_capacity_settings()).is_ok());
            for field in [
                "retention_ms",
                "max_checkpoint_bytes",
                "max_records",
                "max_references",
                "max_tail_inputs",
                "max_tail_bytes",
                "max_input_bytes",
                "prune_batch_rows",
            ] {
                for invalid in [
                    json!(0),
                    json!(-1),
                    json!(true),
                    json!("3"),
                    json!(1.5),
                    Value::Null,
                    json!(u64::MAX),
                ] {
                    let mut value = full_capacity_settings();
                    value["maintenance"][field] = invalid;
                    assert!(
                        check(value).is_err(),
                        "accepted invalid maintenance {field}"
                    );
                }
                let mut value = full_capacity_settings();
                value["maintenance"].as_object_mut().unwrap().remove(field);
                assert!(
                    check(value).is_err(),
                    "accepted missing maintenance {field}"
                );
            }
            let mut value = maintained_settings();
            value["maintenance"]["unknown"] = json!(1);
            assert!(check(value).is_err());
            let mut value = maintained_settings();
            value.as_object_mut().unwrap().remove("bounded_host");
            assert!(check(value).is_err());
            let mut value = maintained_settings();
            value["maintenance"]["max_input_bytes"] = json!(8_000_001);
            assert!(check(value).is_err());
        }

        #[test]
        fn production_journal_wiring_rejects_policy_disable_and_change() {
            struct Scratch(PathBuf);
            impl Drop for Scratch {
                fn drop(&mut self) {
                    let _ = fs::remove_dir_all(&self.0);
                }
            }
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "sandproof-maintenance-config-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&directory).unwrap();
            let scratch = Scratch(directory);
            let database = scratch.0.join("controller.db");
            let config: ConfigFile = serde_json::from_value(full_capacity_settings()).unwrap();
            let host = resolve(full_capacity_settings());
            let policy = config.maintenance.unwrap();
            drop(
                open_journal(
                    database.clone(),
                    &config.namespace,
                    Some(host.clone()),
                    Some(policy.clone()),
                )
                .unwrap(),
            );
            assert!(
                open_journal(
                    database.clone(),
                    &config.namespace,
                    Some(host.clone()),
                    None
                )
                .is_err()
            );
            let mut changed = policy.clone();
            changed.retention_ms += 1;
            assert!(
                ControllerJournal::preflight_maintained(
                    &database,
                    &config.namespace,
                    &host,
                    &changed
                )
                .is_err()
            );
            assert!(
                open_journal(
                    database.clone(),
                    &config.namespace,
                    Some(host.clone()),
                    Some(changed)
                )
                .is_err()
            );
            let reopened = open_journal(
                database,
                &config.namespace,
                Some(host),
                Some(policy.clone()),
            )
            .unwrap();
            assert_eq!(reopened.maintenance_config().unwrap(), Some(policy.clone()));
            assert!(
                open_journal(
                    scratch.0.join("invalid.db"),
                    &config.namespace,
                    None,
                    Some(policy)
                )
                .is_err()
            );
        }

        #[test]
        fn bounded_route_capacity_comes_only_from_explicit_transfer_limits() {
            let original = resolve(settings());
            // The control reserve is already included in the per-VM cap.
            assert_eq!(original.max_data_routes_per_slot, 4);
            assert_eq!(original.max_data_routes_global, 8);
            assert_eq!(original.data_route_control_reserve, 1);
            let mut changed = settings();
            changed["data_transfer"]["max_transfers_per_vm"] = json!(5);
            let changed = resolve(changed);
            assert_eq!(changed.max_data_routes_per_slot, 5);
            assert_ne!(changed, original);
            let mut global_only = settings();
            global_only["data_transfer"]["max_transfers"] = json!(16);
            let global_only = resolve(global_only);
            assert_eq!(global_only.max_data_routes_global, 16);
            assert_eq!(
                global_only.max_data_routes_per_slot,
                original.max_data_routes_per_slot
            );
            assert_ne!(global_only, original);
            let mut reserve_only = settings();
            reserve_only["data_transfer"]["control_reserve"] = json!(2);
            let reserve_only = resolve(reserve_only);
            assert_eq!(reserve_only.data_route_control_reserve, 2);
            assert_ne!(reserve_only, original);
            for changed in [changed, global_only, reserve_only] {
                assert_ne!(
                    serde_json::to_vec(&changed).unwrap(),
                    serde_json::to_vec(&original).unwrap()
                );
            }
            for field in [
                "max_data_routes_per_slot",
                "max_data_routes_global",
                "data_route_control_reserve",
            ] {
                let mut duplicated = settings();
                duplicated["bounded_host"][field] = json!(4);
                assert!(check(duplicated).is_err());
            }
        }

        #[test]
        fn persisted_route_limits_refuse_changed_config_without_changing_the_original() {
            struct Scratch(PathBuf);
            impl Drop for Scratch {
                fn drop(&mut self) {
                    let _ = fs::remove_dir_all(&self.0);
                }
            }
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "sandproof-route-config-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            let scratch = Scratch(path);
            let database = scratch.0.join("controller.db");
            let namespace = "0123456789abcdef";
            let original = resolve(settings());
            drop(
                ControllerJournal::open_bounded(&database, namespace, original.clone(), 1).unwrap(),
            );
            for (field, value) in [
                ("max_transfers_per_vm", 5),
                ("max_transfers", 16),
                ("control_reserve", 2),
            ] {
                let mut changed = settings();
                changed["data_transfer"][field] = json!(value);
                let changed = resolve(changed);
                assert!(
                    ControllerJournal::preflight_bounded(&database, namespace, &changed).is_err()
                );
                assert!(ControllerJournal::open_bounded(&database, namespace, changed, 2).is_err());
                let reopened =
                    ControllerJournal::open_bounded(&database, namespace, original.clone(), 2)
                        .unwrap();
                assert_eq!(
                    reopened.bounded_host_config().unwrap(),
                    Some(original.clone())
                );
            }
        }

        #[test]
        fn bounded_mode_rejects_missing_or_invalid_transfer_budgets() {
            let mut missing = settings();
            missing.as_object_mut().unwrap().remove("data_transfer");
            assert!(check(missing.clone()).is_err());
            missing.as_object_mut().unwrap().remove("bounded_host");
            assert!(check(missing).is_ok());
            let mut absent = settings();
            absent["data_transfer"] = Value::Null;
            assert!(check(absent).is_err());
            for (field, invalid) in [
                ("max_transfers_per_vm", json!(0)),
                ("max_transfers_per_vm", json!(true)),
                ("max_transfers_per_vm", json!(9)),
                ("max_transfers", json!(3)),
                ("control_reserve", json!(0)),
                ("control_reserve", json!(4)),
                ("rpc_request_bytes", json!(8 * 1024 * 1024 + 1)),
                ("file_upload_bytes", json!(0)),
                ("request_idle_seconds", json!(0)),
            ] {
                let mut value = settings();
                value["data_transfer"][field] = invalid;
                assert!(check(value).is_err(), "accepted invalid {field}");
            }
        }

        fn context(vm: u64, previous: Option<u64>) -> HostSlotContext {
            HostSlotContext {
                position: HostSlotId(0),
                expected_previous: previous.map(core::VmId),
                binding: HostBinding {
                    owner: core::TenantId(1),
                    sandbox: core::SandboxId(vm),
                    generation: 1,
                    vm: core::VmId(vm),
                    network: core::NetSlotId(vm),
                },
            }
        }

        #[test]
        fn bounded_configuration_is_explicit_and_permanent_ids_do_not_expand_uid_span() {
            assert!(check(settings()).is_ok());
            let mut bounded = settings();
            bounded["max_vm_id"] = json!(0);
            assert!(check(bounded).is_ok());
            let mut legacy = settings();
            legacy.as_object_mut().unwrap().remove("bounded_host");
            assert!(
                serde_json::from_value::<ConfigFile>(legacy)
                    .unwrap()
                    .bounded_host
                    .is_none()
            );
            let mut value = settings();
            value["bounded_host"] = Value::Null;
            assert!(
                serde_json::from_value::<ConfigFile>(value)
                    .unwrap()
                    .bounded_host
                    .is_none()
            );
            let mut value = settings();
            value["uid_base"] = json!(u32::MAX - 2);
            value["bounded_host"]["uid_base"] = json!(u32::MAX - 2);
            assert!(check(value.clone()).is_ok());
            value["bounded_host"]["maintenance"]["max_host_slots"] = json!(3);
            assert!(check(value).is_err());
        }

        #[test]
        fn bounded_configuration_rejects_missing_unknown_untyped_and_invalid_limits() {
            let paths = [
                "/uid_base",
                "/bounded_host/uid_base",
                "/bounded_host/maintenance/max_retained_results",
                "/bounded_host/maintenance/max_host_slots",
                "/bounded_host/maintenance/windows/max_open_windows",
                "/bounded_host/maintenance/windows/max_duration_ms",
            ];
            for path in paths {
                for invalid in [
                    json!(0),
                    json!(-1),
                    json!(true),
                    json!("2"),
                    json!(1.5),
                    Value::Null,
                ] {
                    let mut value = settings();
                    *value.pointer_mut(path).unwrap() = invalid;
                    assert!(check(value).is_err(), "accepted invalid {path}");
                }
                let mut value = settings();
                let (parent, field) = path.rsplit_once('/').unwrap();
                value
                    .pointer_mut(parent)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert!(check(value).is_err(), "accepted missing {path}");
            }
            for path in [
                "/bounded_host",
                "/bounded_host/maintenance",
                "/bounded_host/maintenance/windows",
            ] {
                let mut value = settings();
                value
                    .pointer_mut(path)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert("misspelled".into(), json!(1));
                assert!(check(value).is_err(), "accepted unknown field in {path}");
            }
            for invalid in [
                json!(-1),
                json!(true),
                json!("0"),
                json!(1.5),
                json!(1_048_575),
                Value::Null,
            ] {
                let mut value = settings();
                value["bounded_host"]["uplink_base"] = invalid;
                assert!(check(value).is_err());
            }
            let mut value = settings();
            value["bounded_host"]
                .as_object_mut()
                .unwrap()
                .remove("uplink_base");
            assert!(check(value).is_err());
            let mut value = settings();
            value["bounded_host"]["uid_base"] = json!(1001);
            assert!(check(value).is_err());
            let mut value = settings();
            value["bounded_host"]["maintenance"]["max_host_slots"] = json!(1_048_577);
            assert!(check(value).is_err());
        }

        fn audit(
            durable: &[(HostSlotContext, HostSlotPhase)],
            observed: &[(HostSlotContext, HostSlotPhase)],
        ) -> Result<(), Box<dyn Error>> {
            audit_host_slot_contexts(durable, observed, |_| true)
        }

        #[test]
        fn startup_reopens_only_terminal_fence_without_any_launch_dispatch() {
            use sandproof_controller_service::{NeverCrash, kill_v1};
            struct Scratch(PathBuf);
            impl Drop for Scratch {
                fn drop(&mut self) {
                    let _ = fs::remove_dir_all(&self.0);
                }
            }
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "sandproof-cancel-binding-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&directory).unwrap();
            let scratch = Scratch(directory);
            let config: ConfigFile = serde_json::from_value(settings()).unwrap();
            let bounded = resolve(settings());
            for claimed_opening_fence in [false, true] {
                let database = scratch.0.join(format!("{claimed_opening_fence}.db"));
                let mut journal = ControllerJournal::open_bounded(
                    &database,
                    &config.namespace,
                    bounded.clone(),
                    0,
                )
                .unwrap();
                journal
                    .admit_create(
                        core::CreateRequest {
                            actor: core::Actor::Tenant(core::TenantId(1)),
                            owner: core::TenantId(1),
                            sandbox: core::SandboxId(1),
                            workspace: core::FsId(1),
                            file_policy: core::FilePolicyRef {
                                digest: core::PolicyDigest(1),
                            },
                            policy: core::NetworkPolicy::DenyAll,
                            operation: core::OperationId(1),
                        },
                        0,
                        60,
                        1024,
                        managed_config(&config, 1024).unwrap().limits,
                        &mut NeverCrash,
                    )
                    .unwrap();
                if claimed_opening_fence {
                    assert!(matches!(
                        journal
                            .dispatch_next::<HostObservation, _>(
                                &mut |_| Err(WorkerTransportError::ConnectionLost),
                                &mut NeverCrash
                            )
                            .unwrap(),
                        sandproof_controller_service::DispatchResult::OutcomeUnknown { .. }
                    ));
                }
                assert!(
                    kill_v1::<HostObservation, _>(
                        &mut journal,
                        core::KillRequest {
                            actor: core::Actor::Tenant(core::TenantId(1)),
                            sandbox: core::SandboxId(1),
                            operation: core::OperationId(2),
                        },
                        &mut |_| Err(WorkerTransportError::ConnectionLost)
                    )
                    .is_err()
                );
                drop(journal);
                let journal = ControllerJournal::open_bounded(
                    &database,
                    &config.namespace,
                    bounded.clone(),
                    1,
                )
                .unwrap();
                let world = journal.world().unwrap();
                let durable = journal.host_slot_inventory().unwrap();
                assert_eq!(durable.len(), 1);
                let audit = audit_host_slot_contexts(&durable, &[], |context| {
                    can_install_host_slot_binding(&world, context.binding.sandbox)
                });
                assert_eq!(audit.is_ok(), !claimed_opening_fence);
                assert_eq!(journal.next_sandbox_id().unwrap().0, 2);
                let record = world.sandbox_record(core::SandboxId(1)).unwrap();
                assert!(record.current_vm.is_none());
                if !claimed_opening_fence {
                    assert!(record.terminal);
                    assert_eq!(record.phase, core::Phase::Revoking);
                }
            }
        }

        #[test]
        fn startup_never_reconstructs_a_binding_after_host_work_was_claimed() {
            use HostSlotPhase::{Free, Occupied};
            let first = context(0, None);
            let second = context(1, Some(0));
            assert!(audit_host_slot_contexts(&[(first, Occupied)], &[], |_| false).is_err());
            assert!(
                audit_host_slot_contexts(&[(second, Occupied)], &[(first, Free)], |_| false)
                    .is_err()
            );
            assert!(
                audit_host_slot_contexts(&[(second, Occupied)], &[(second, Occupied)], |_| false)
                    .is_ok()
            );
        }

        #[test]
        fn startup_accepts_only_precise_bind_and_release_transaction_lag() {
            use HostSlotPhase::{Free, Occupied, Releasing};
            let first = context(0, None);
            assert!(audit(&[(first, Occupied)], &[]).is_ok());
            for phase in [Occupied, Releasing, Free] {
                assert!(audit(&[(first, phase)], &[(first, phase)]).is_ok());
            }
            for phase in [Occupied, Releasing, Free] {
                assert!(audit(&[(first, Releasing)], &[(first, phase)]).is_ok());
            }
            let second = context(1, Some(0));
            assert!(audit(&[(second, Occupied)], &[(first, Free)]).is_ok());
            assert!(audit(&[(second, Occupied)], &[]).is_err());
            assert!(audit(&[(second, Occupied)], &[(first, Occupied)]).is_err());
            assert!(audit(&[(second, Occupied)], &[(first, Releasing)]).is_err());
            assert!(audit(&[(second, Free)], &[(second, Releasing)]).is_err());
            assert!(audit(&[(second, Free)], &[]).is_err());
            assert!(audit(&[(second, Releasing)], &[]).is_err());
            assert!(audit(&[(second, Occupied)], &[(second, Releasing)]).is_err());
            assert!(audit(&[], &[(first, Free)]).is_err());
        }

        #[test]
        fn startup_rejects_context_corruption_duplicates_and_stale_rebinding() {
            use HostSlotPhase::{Free, Occupied, Releasing};
            let old = context(1, None);
            let fresh = context(2, Some(1));
            for altered in [
                HostSlotContext {
                    expected_previous: Some(core::VmId(99)),
                    ..fresh
                },
                HostSlotContext {
                    binding: HostBinding {
                        owner: core::TenantId(2),
                        ..fresh.binding
                    },
                    ..fresh
                },
                HostSlotContext {
                    binding: HostBinding {
                        network: core::NetSlotId(99),
                        ..fresh.binding
                    },
                    ..fresh
                },
            ] {
                for phase in [Occupied, Releasing, Free] {
                    assert!(audit(&[(fresh, phase)], &[(altered, phase)]).is_err());
                }
            }
            assert!(audit(&[(context(0, Some(1)), Occupied)], &[(old, Free)]).is_err());
            assert!(audit(&[(context(1, Some(1)), Occupied)], &[(old, Free)]).is_err());
            assert!(audit(&[(fresh, Occupied)], &[(fresh, Occupied); 2]).is_err());
            assert!(audit(&[(fresh, Occupied); 2], &[(fresh, Occupied)]).is_err());
            let same_vm = HostSlotContext {
                position: HostSlotId(1),
                ..fresh
            };
            assert!(audit(&[(fresh, Occupied), (same_vm, Occupied)], &[]).is_err());
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args_os().nth(1) {
        Some(arg) if arg == "relay" => gateway::relay(false).await,
        Some(arg) if arg == "relay-bounded" => gateway::relay(true).await,
        _ => linux::run().await,
    }
}
