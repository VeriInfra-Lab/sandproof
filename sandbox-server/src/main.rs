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

    use sandbox_controller_core as core;
    use sandproof_api::{
        ApiCredential, PortError,
        managed::{HostAdapter, ManagedConfig, ManagedPort},
        router,
    };
    use sandproof_controller_service::{
        AdmissionLimits, ControllerJournal, CreateOptions, WorkerTransportError,
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
    }

    fn default_timeout_secs() -> u64 {
        sandproof_controller_service::DEFAULT_TIMEOUT_SECS
    }

    fn default_startup_timeout_secs() -> u64 {
        sandproof_controller_service::DEFAULT_STARTUP_TIMEOUT_SECS
    }

    pub(crate) struct NodeHost(LinuxWorker);

    impl HostAdapter for NodeHost {
        type Evidence = HostObservation;

        fn validate_template(&self, template: &TemplateBinding) -> Result<(), PortError> {
            self.0
                .validate_template(template)
                .map_err(|_| PortError::Busy)
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
        let world = journal.world()?;
        let uncertain = journal.uncertain_actions()?;
        for slot in inventory.slots {
            audit_host_slot(node, journal, &world, &uncertain, &slot)?;
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
                    let recovery = port.recover_on_startup()?;
                    let expiry = port.expire_due_report()?;
                    Ok::<_, PortError>((recovery, expiry))
                })
                .await
                {
                    Ok(Ok((recovery, expiry)))
                        if recovery.failed_sandboxes > 0 || expiry.unresolved_sandboxes > 0 =>
                    {
                        if last_failure_log
                            .is_none_or(|instant| instant.elapsed() >= Duration::from_mins(1))
                        {
                            eprintln!(
                                "reconciliation parked {} sandboxes after host or receipt errors; {} due expiries remain unresolved; manual inspection may be required",
                                recovery.failed_sandboxes, expiry.unresolved_sandboxes
                            );
                            last_failure_log = Some(Instant::now());
                        }
                    }
                    Ok(Ok(_)) => last_failure_log = None,
                    Ok(Err(_)) => eprintln!("reconciliation or expiry worker needs attention"),
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
        let managed = managed_config(&config, disk_bytes)?;
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
            host_resources: config.host_resources.clone(),
            file_policy_digest: core::PolicyDigest(config.file_policy_digest),
        })?;
        let journal = ControllerJournal::open(config.database, &config.namespace)?;
        if journal.has_template_registry()? && config.template_registry.is_none() {
            return Err("durable template registry cannot be disabled for this journal".into());
        }
        audit_host_inventory(&node, &journal)?;
        let port = if let Some(catalog) = config.templates {
            ManagedPort::with_templates(journal, NodeHost(node), managed, catalog)
                .map_err(|_| "invalid prebuilt template configuration")?
        } else {
            ManagedPort::new(journal, NodeHost(node), managed)
        };
        let port = Arc::new(if let Some(registry) = config.template_registry {
            port.with_registry(
                sandproof_controller_service::template_registry::CandidateStore::new(registry, 0)?,
            )
            .map_err(|_| "invalid durable template registry configuration")?
        } else {
            port
        });
        let recovery = port.recover_on_startup().map_err(|error| {
            std::io::Error::other(format!("startup recovery failed: {error:?}"))
        })?;
        if recovery.unresolved_sandboxes != 0 {
            eprintln!(
                "startup recovery left {} sandboxes unresolved ({} after host or receipt errors); no uncertain host action was repeated",
                recovery.unresolved_sandboxes, recovery.failed_sandboxes
            );
        }
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
