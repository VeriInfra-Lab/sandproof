//! Host worker adaptation and deterministic debug-only fault boundaries.

use crate::fault::{busy, project};
use crate::{HostObservation, LinuxWorker, PhysicalProfileLease};
use sandbox_controller_core as core;
use sandproof_controller_service::application::fault::{FaultContext, FaultStage};
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
                project(
                    FaultStage::Observe,
                    FaultContext::action(action),
                    error.as_ref(),
                )
                .emit();
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
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
    }

    fn reclaim_incomplete_network(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_network(action)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
    }

    fn reclaim_incomplete_activation(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_activation(action)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
    }

    fn reclaim_incomplete_terminal_fence(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_terminal_fence(action)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
    }

    fn reclaim_incomplete_open_fence(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_open_fence(action)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
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
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
    }

    fn reclaim_incomplete_abort_prepare(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_abort_prepare(action)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
    }

    fn reclaim_incomplete_cleanup(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .reclaim_incomplete_cleanup(action)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::Reclaim,
                    FaultContext::action(action),
                )
            })
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
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::BindSlot,
                    FaultContext::slot(context),
                )
            })
    }

    fn release_host_slot(&mut self, context: &HostSlotContext) -> Result<(), ApplicationError> {
        self.worker.release_host_slot(context).map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::ReleaseSlot,
                FaultContext::slot(context),
            )
        })
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
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::DataGate,
                    FaultContext {
                        vm: Some(vm.0),
                        generation: Some(generation),
                        ..FaultContext::instance(sandbox, None)
                    },
                )
            })
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
                project(
                    FaultStage::Relay,
                    FaultContext {
                        vm: Some(vm.0),
                        generation: Some(generation),
                        ..FaultContext::instance(sandbox, None)
                    },
                    error.as_ref(),
                )
                .emit();
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
        self.worker.data_gate_drained(context).map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::DataGate,
                FaultContext::slot(context),
            )
        })
    }
}

impl HostCapacity for NodeHost {
    fn available_compute(&self) -> Result<(u64, u64), ApplicationError> {
        crate::host_capacity::available_compute().map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::PhysicalStorage,
                FaultContext::default(),
            )
        })
    }
    fn available_disk_bytes(&self) -> Result<u64, ApplicationError> {
        self.worker.available_disk_bytes().map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::DiskCapacity,
                FaultContext::default(),
            )
        })
    }

    fn validate_physical_storage(
        &self,
        profile: &sandproof_controller_service::physical_storage::PhysicalStorageProfile,
        pool: &sandproof_controller_service::physical_storage::PhysicalPool,
    ) -> Result<(), ApplicationError> {
        self.physical_storage
            .validate_admission(profile, pool)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::PhysicalStorage,
                    FaultContext::default(),
                )
            })
    }

    fn public_ipv4_ready(&self) -> Result<bool, ApplicationError> {
        std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
            .map(|value| value.trim() == "1")
            .map_err(|error| busy(&error, FaultStage::PublicIpv4, FaultContext::default()))
    }
}

impl LaunchConfiguration for NodeHost {
    fn put_secret(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
    ) -> Result<(), ApplicationError> {
        self.worker.put_secret(sandbox, token).map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::PutCredential,
                FaultContext::instance(sandbox, None),
            )
        })
    }

    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, ApplicationError> {
        self.worker.has_secret(sandbox).map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::CheckCredential,
                FaultContext::instance(sandbox, None),
            )
        })
    }

    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, ApplicationError> {
        self.worker.token_for(sandbox).map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::ReadCredential,
                FaultContext::instance(sandbox, None),
            )
        })
    }

    fn put_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        env_vars: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), ApplicationError> {
        self.worker
            .put_launch_secret(sandbox, token, env_vars)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::PutCredential,
                    FaultContext::instance(sandbox, None),
                )
            })
    }

    fn has_launch_config(
        &self,
        sandbox: core::SandboxId,
        env_vars: &std::collections::BTreeMap<String, String>,
    ) -> Result<bool, ApplicationError> {
        self.worker
            .has_launch_secret(sandbox, env_vars)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::CheckCredential,
                    FaultContext::instance(sandbox, None),
                )
            })
    }

    fn put_bound_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        options: &CreateOptions,
    ) -> Result<(), ApplicationError> {
        self.worker
            .put_bound_launch_secret(sandbox, token, options)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::PutCredential,
                    FaultContext::instance(sandbox, None),
                )
            })
    }

    fn has_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        options: &CreateOptions,
    ) -> Result<bool, ApplicationError> {
        self.worker
            .has_bound_launch_secret(sandbox, options)
            .map_err(|error| {
                busy(
                    error.as_ref(),
                    FaultStage::CheckCredential,
                    FaultContext::instance(sandbox, None),
                )
            })
    }
}

impl TemplateArtifacts for NodeHost {
    fn validate_template(&self, template: &TemplateBinding) -> Result<(), ApplicationError> {
        self.worker.validate_template(template).map_err(|error| {
            busy(
                error.as_ref(),
                FaultStage::TemplateValidation,
                FaultContext::default(),
            )
        })
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
