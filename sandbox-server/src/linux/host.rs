//! Host worker adaptation and deterministic debug-only fault boundaries.

use sandbox_controller_core as core;
use sandproof_controller_service::application::{ApplicationError as PortError, HostAdapter};
use sandproof_controller_service::{
    CreateOptions, HostSlotContext, WorkerTransportError, template_registry::CandidateVerification,
    templates::TemplateBinding,
};
use sandproof_node::{HostObservation, LinuxWorker};
#[cfg(debug_assertions)]
use sandproof_protocol::WorkOutcome;
use sandproof_protocol::{WorkAction, WorkReceipt};
#[cfg(debug_assertions)]
use std::{fs, os::unix::fs::MetadataExt, path::PathBuf};

pub(crate) struct NodeHost(
    pub(super) LinuxWorker,
    pub(super) crate::storage::PhysicalProfileLease,
);

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
            .map(Some)
            .map_err(|_| PortError::Busy)
    }

    fn spawn_data_relay(
        &mut self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
        control: bool,
    ) -> Result<std::process::Child, PortError> {
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

    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>> {
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
