//! Host worker adaptation and deterministic debug-only fault boundaries.

use crate::{HostObservation, LinuxWorker, PhysicalProfileLease};
use sandbox_controller_core as core;
use sandproof_controller_service::application::{
    ApplicationError, DataRelay, HostCapacity, HostSlots, LaunchConfiguration, TemplateArtifacts,
    WorkEvidence, WorkExecution, WorkRecovery,
};
use sandproof_controller_service::{
    CreateOptions, HostSlotContext, WorkerTransportError, template_registry::CandidateVerification,
    templates::TemplateBinding,
};
use sandproof_protocol::{WorkAction, WorkReceipt};
#[cfg(debug_assertions)]
mod faults;

/// Production adapter: one worker, physical lease and trusted relay executable.
pub struct NodeHost {
    worker: LinuxWorker,
    physical_storage: PhysicalProfileLease,
    relay_executable: std::path::PathBuf,
}
impl NodeHost {
    /// The relay executable is selected only by trusted service assembly.
    #[must_use]
    pub fn new(
        worker: LinuxWorker,
        physical_storage: PhysicalProfileLease,
        relay_executable: std::path::PathBuf,
    ) -> Self {
        Self {
            worker,
            physical_storage,
            relay_executable,
        }
    }
}

impl WorkEvidence for NodeHost {
    type Evidence = HostObservation;
}
impl WorkExecution for NodeHost {
    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError> {
        #[cfg(debug_assertions)]
        faults::before_execute(action);
        let result = self.worker.execute(action);
        #[cfg(debug_assertions)]
        faults::after_execute(action, &result);
        result
    }
}

impl WorkRecovery for NodeHost {
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>> {
        match self.worker.observe_action(action) {
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
    ) -> Result<(), ApplicationError> {
        #[cfg(debug_assertions)]
        if faults::refuse_reclaim(action, "SANDPROOF_TEST_FAIL_RECLAIM_PREPARE_SANDBOX") {
            // Refuse only this sandbox's compensating observation. The
            // original PrepareVm remains unknown; no launch is replayed.
            return Err(ApplicationError::Busy);
        }
        self.worker
            .reclaim_incomplete_prepare(action)
            .map_err(|_| ApplicationError::Busy)
    }

    fn reclaim_incomplete_network(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_network(action)
            .map_err(|_| ApplicationError::Busy)
    }

    fn reclaim_incomplete_activation(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_activation(action)
            .map_err(|_| ApplicationError::Busy)
    }

    fn reclaim_incomplete_terminal_fence(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_terminal_fence(action)
            .map_err(|_| ApplicationError::Busy)
    }

    fn reclaim_incomplete_open_fence(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_open_fence(action)
            .map_err(|_| ApplicationError::Busy)
    }

    fn reclaim_incomplete_stop(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        #[cfg(debug_assertions)]
        if faults::refuse_reclaim(action, "SANDPROOF_TEST_FAIL_RECLAIM_STOP_SANDBOX") {
            // Test-only refusal before host I/O. The old stop remains
            // unknown, so another sandbox must still be able to recover.
            return Err(ApplicationError::Busy);
        }
        self.worker
            .reclaim_incomplete_stop(action)
            .map_err(|_| ApplicationError::Busy)
    }

    fn reclaim_incomplete_abort_prepare(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_abort_prepare(action)
            .map_err(|_| ApplicationError::Busy)
    }

    fn reclaim_incomplete_cleanup(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_cleanup(action)
            .map_err(|_| ApplicationError::Busy)
    }
}

impl HostSlots for NodeHost {
    fn bind_host_slot(
        &mut self,
        context: &HostSlotContext,
        allow_install: bool,
    ) -> Result<(), ApplicationError> {
        self.worker
            .bind_host_slot(context, allow_install)
            .map_err(|_| ApplicationError::Busy)
    }

    fn release_host_slot(&mut self, context: &HostSlotContext) -> Result<(), ApplicationError> {
        self.worker
            .release_host_slot(context)
            .map_err(|_| ApplicationError::Busy)
    }
}

impl DataRelay for NodeHost {
    fn acquire_data_gate(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
    ) -> Result<Option<std::fs::File>, ApplicationError> {
        self.worker
            .acquire_data_gate(sandbox, vm, generation)
            .map(Some)
            .map_err(|_| ApplicationError::Busy)
    }

    fn spawn_data_relay(
        &mut self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
        control: bool,
    ) -> Result<std::process::Child, ApplicationError> {
        self.worker
            .spawn_data_relay(sandbox, vm, generation, control, &self.relay_executable)
            .map_err(|error| {
                if error
                    .downcast_ref::<crate::RouteCapacityExceeded>()
                    .is_some()
                {
                    ApplicationError::QuotaExceeded
                } else {
                    ApplicationError::Busy
                }
            })
    }

    fn data_gate_drained(&self, context: &HostSlotContext) -> Result<bool, ApplicationError> {
        self.worker
            .data_gate_drained(context)
            .map_err(|_| ApplicationError::Busy)
    }
}

impl HostCapacity for NodeHost {
    fn available_disk_bytes(&self) -> Result<u64, ApplicationError> {
        self.worker
            .available_disk_bytes()
            .map_err(|_| ApplicationError::Busy)
    }

    fn validate_physical_storage(
        &self,
        profile: &sandproof_controller_service::physical_storage::PhysicalStorageProfile,
        pool: &sandproof_controller_service::physical_storage::PhysicalPool,
    ) -> Result<(), ApplicationError> {
        self.physical_storage
            .validate_admission(profile, pool)
            .map_err(|_| ApplicationError::Busy)
    }

    fn public_ipv4_ready(&self) -> Result<bool, ApplicationError> {
        std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
            .map(|value| value.trim() == "1")
            .map_err(|_| ApplicationError::Busy)
    }
}

impl LaunchConfiguration for NodeHost {
    fn put_secret(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
    ) -> Result<(), ApplicationError> {
        self.worker
            .put_secret(sandbox, token)
            .map_err(|_| ApplicationError::Busy)
    }

    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, ApplicationError> {
        self.worker
            .has_secret(sandbox)
            .map_err(|_| ApplicationError::Busy)
    }

    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, ApplicationError> {
        self.worker
            .token_for(sandbox)
            .map_err(|_| ApplicationError::Busy)
    }

    fn put_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        env_vars: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .put_launch_secret(sandbox, token, env_vars)
            .map_err(|_| ApplicationError::Busy)
    }

    fn has_launch_config(
        &self,
        sandbox: core::SandboxId,
        env_vars: &std::collections::BTreeMap<String, String>,
    ) -> Result<bool, ApplicationError> {
        self.worker
            .has_launch_secret(sandbox, env_vars)
            .map_err(|_| ApplicationError::Busy)
    }

    fn put_bound_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        options: &CreateOptions,
    ) -> Result<(), ApplicationError> {
        self.worker
            .put_bound_launch_secret(sandbox, token, options)
            .map_err(|_| ApplicationError::Busy)
    }

    fn has_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        options: &CreateOptions,
    ) -> Result<bool, ApplicationError> {
        self.worker
            .has_bound_launch_secret(sandbox, options)
            .map_err(|_| ApplicationError::Busy)
    }
}

impl TemplateArtifacts for NodeHost {
    fn validate_template(&self, template: &TemplateBinding) -> Result<(), ApplicationError> {
        self.worker
            .validate_template(template)
            .map_err(|_| ApplicationError::Busy)
    }

    fn cache_candidate_verification(
        &self,
        template: &TemplateBinding,
        verification: &CandidateVerification,
    ) {
        self.worker
            .cache_candidate_verification(template, verification);
    }
}
