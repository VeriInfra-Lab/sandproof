//! Host worker adaptation and deterministic debug-only fault boundaries.

use crate::fault::{busy, project};
use crate::{HostObservation, LinuxWorker, PhysicalProfileLease, ProtectedHostEpochSource};
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
    host_epoch_source: Option<ProtectedHostEpochSource>,
    capture_profile: Option<crate::linux::ProtectedCaptureProfile>,
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
            host_epoch_source: None,
            capture_profile: None,
        }
    }
    /// Select only the original protected registration path. This does not
    /// establish an epoch; every bootstrap invocation checks its declared fields.
    #[must_use]
    pub fn with_host_epoch_source(mut self, source: ProtectedHostEpochSource) -> Self {
        self.host_epoch_source = Some(source);
        self
    }
}

impl NodeHost {
    /// Pause only the source of an already committed and claimed `FreezeVm`.
    /// The service independently binds the full external operation to this
    /// native action and supplies the requirement from the original saved work.
    /// # Errors
    /// Any failure may follow a physical Pause. Keep the original work/VM;
    /// never translate the error into no effect, retry or Stop permission.
    pub fn pause_original(
        &self,
        action: &WorkAction<core::EffectId>,
        request: &sandproof_controller_service::capture::PauseWorkRequest,
        original_source: &sandproof_controller_service::capture::SourceObservationRequirement,
    ) -> Result<HostObservation, Box<dyn std::error::Error>> {
        self.worker.pause_original(
            self.host_epoch_source
                .as_ref()
                .ok_or("missing original protected bootstrap source")?,
            action,
            request,
            original_source,
        )
    }

    /// Observe the same original Pause without reissuing PATCH or Full.
    /// A current Paused observation does not prove a prior Full worker ended.
    /// # Errors
    /// Running, exited, foreign and unknown sources remain unresolved here;
    /// no cleanup, resource refund or Stop permission follows.
    pub fn observe_pause_original(
        &self,
        action: &WorkAction<core::EffectId>,
        request: &sandproof_controller_service::capture::PauseWorkRequest,
        original_source: &sandproof_controller_service::capture::SourceObservationRequirement,
    ) -> Result<HostObservation, Box<dyn std::error::Error>> {
        self.worker.observe_pause_original(
            self.host_epoch_source
                .as_ref()
                .ok_or("missing original protected bootstrap source")?,
            action,
            request,
            original_source,
        )
    }
}

impl NodeHost {
    /// Assemble the one adapter-held profile from protected server settings and
    /// the ORIGINAL journal epoch. This only reopens an existing layout; it
    /// never creates a new layout or takes settings from an incoming work DTO.
    /// Copy caps do not grant a durable claim, reservation or permission to run.
    /// # Errors
    /// Missing/changed protected source, epoch or layout refuses assembly.
    pub fn with_capture_settings(
        mut self,
        settings: crate::ProtectedCaptureSettings,
        original: &sandproof_controller_service::capture::HostEpochRequirement,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let provider = self
            .host_epoch_source
            .as_ref()
            .ok_or("missing original protected bootstrap source")?;
        self.capture_profile = Some(crate::linux::ProtectedCaptureProfile::assemble(
            &self.worker,
            provider,
            settings,
            original,
        )?);
        Ok(self)
    }

    /// Propose parameters from the original sealed source and local profile.
    /// The service must persist/check its original plan and budgets before
    /// dispatching any physical Full. This proposal is never a permission.
    /// # Errors
    /// Missing profile, incompatible source or unknown facts remain errors.
    pub fn prepare_full_capture(
        &self,
        pause: &sandproof_controller_service::capture::PauseWorkRequest,
        original: &sandproof_controller_service::capture::SourceObservationRequirement,
    ) -> Result<
        sandproof_controller_service::capture::FullCaptureWorkRequest,
        Box<dyn std::error::Error>,
    > {
        self.worker.prepare_full_capture(
            self.host_epoch_source
                .as_ref()
                .ok_or("missing original protected bootstrap source")?,
            self.capture_profile
                .as_ref()
                .ok_or("protected Full profile not assembled")?,
            pause,
            original,
        )
    }

    /// Execute only an originally claimed Full with independently checked local
    /// parameters. The original V5 claim/resource plan and closed/drained V8
    /// access are required; no untrusted DTO can install the private profile.
    /// # Errors
    /// Every error may follow actual Full/Seal or publication. Preserve original
    /// Dispatched/Unknown Full concurrency and material responsibility: callback
    /// return, Paused/Exited or a directory does NOT prove the FC writer ended.
    pub fn capture_original(
        &self,
        action: &WorkAction<core::EffectId>,
        request: &sandproof_controller_service::capture::FullCaptureWorkRequest,
        original: &sandproof_controller_service::capture::SourceObservationRequirement,
    ) -> Result<crate::full_snapshot::FullCaptureObservation, Box<dyn std::error::Error>> {
        self.worker.capture_original(
            self.host_epoch_source
                .as_ref()
                .ok_or("missing original protected bootstrap source")?,
            self.capture_profile
                .as_ref()
                .ok_or("protected Full profile not assembled")?,
            action,
            request,
            original,
        )
    }

    /// Read only the exact saved key. Returned file facts have no captured-source
    /// provenance, durable handoff, worker-end, Stop or resource-release claim.
    /// # Errors
    /// Missing, foreign or incomplete materials remain unresolved; no retry Full.
    pub fn observe_full_capture(
        &self,
        request: &sandproof_controller_service::capture::FullCaptureWorkRequest,
        original: &sandproof_controller_service::capture::SourceObservationRequirement,
        manifest: Option<sandproof_protocol::lifecycle::ContentDigest>,
    ) -> Result<crate::full_snapshot::CheckedStoredCapture, Box<dyn std::error::Error>> {
        self.worker.observe_full_capture(
            self.host_epoch_source
                .as_ref()
                .ok_or("missing original protected bootstrap source")?,
            self.capture_profile
                .as_ref()
                .ok_or("protected Full profile not assembled")?,
            request,
            original,
            manifest.map(|digest| *digest.as_bytes()),
        )
    }
}

impl NodeHost {
    /// Reopen only the original journal's committed material expectation.
    /// This does not consult an old position, launch credential or live VMM.
    /// The journal provider is not implemented yet; this API grants no load,
    /// delete, pin, access or resource-release permission.
    /// # Errors
    /// Missing protected profile, wrong epoch, altered or incomplete bytes
    /// remain errors. No physical lifecycle action or repair follows.
    pub fn observe_committed_capture(
        &self,
        expected: &sandproof_controller_service::capture::CommittedCaptureExpectation,
    ) -> Result<crate::full_snapshot::CheckedStoredCapture, Box<dyn std::error::Error>> {
        crate::linux::observe_committed_capture(
            self.capture_profile
                .as_ref()
                .ok_or("protected Full profile not assembled")?,
            expected,
        )
    }
}

impl WorkEvidence for NodeHost {
    type Evidence = HostObservation;
}
impl WorkExecution for NodeHost {
    fn execute(
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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

    fn release_host_slot(&self, context: &HostSlotContext) -> Result<(), ApplicationError> {
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
        &self,
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
    fn observe_host_epoch(
        &self,
        requirement: &sandproof_controller_service::capture::HostEpochRequirement,
    ) -> Result<sandproof_controller_service::capture::HostEpochObservation, WorkerTransportError>
    {
        let result = self
            .host_epoch_source
            .as_ref()
            .ok_or(sandbox_runtime::RuntimeError::InvalidConfiguration(
                "protected bootstrap source missing",
            ))
            .and_then(|source| source.observe(requirement, self.worker.host_bootstrap_inputs()));
        result.map_err(|error| {
            project(FaultStage::Observe, FaultContext::default(), &error).emit();
            WorkerTransportError::ConnectionLost
        })
    }
    fn observe_lifecycle_source(
        &self,
        requirement: &sandproof_controller_service::capture::SourceObservationRequirement,
    ) -> Result<sandproof_controller_service::capture::RawSourceObservation, WorkerTransportError>
    {
        crate::source_observation::observe(
            &self.worker,
            self.host_epoch_source.as_ref(),
            requirement,
        )
        .map_err(|error| {
            project(FaultStage::Observe, FaultContext::default(), error.as_ref()).emit();
            WorkerTransportError::ConnectionLost
        })
    }
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
    fn put_secret(&self, sandbox: core::SandboxId, token: &str) -> Result<(), ApplicationError> {
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
        &self,
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
        &self,
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
