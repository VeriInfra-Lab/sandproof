//! Original application fixture: controlled host effects, not a second controller.
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
pub mod wire;
#[allow(
    dead_code,
    reason = "original worker helpers serve several component tests"
)]
#[path = "mod.rs"]
mod workflow;
use sandbox_controller_core as core;
use sandproof_controller_service::{
    CreateOptions, HostSlotContext, InternalInboxConfig, InternalTemplate, NeverCrash,
    WorkerTransportError,
    application::{
        ApplicationConfig, ApplicationError, Clock, ControllerApplication, DataRelay, HostCapacity,
        HostSlots, LaunchConfiguration, TemplateArtifacts, WorkEvidence, WorkExecution,
        WorkRecovery,
    },
    physical_storage::{PhysicalPool, PhysicalStorageProfile},
    template_registry::CandidateVerification,
    templates::{ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding},
};
use sandproof_protocol::{
    WorkAction, WorkReceipt,
    internal::lifecycle::{Intent, LifecycleRequest},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
        mpsc,
    },
};
pub use workflow::complete_fixture;

pub struct TestClock(pub AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> Result<i64, ApplicationError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub enum TestEpoch {
    #[default]
    Unavailable,
    Matching,
    WrongBoot,
}

pub type SourceProbe = Box<
    dyn FnOnce(
            &sandproof_controller_service::capture::SourceObservationRequirement,
        ) -> Result<
            sandproof_controller_service::capture::RawSourceObservation,
            WorkerTransportError,
        > + Send,
>;

#[derive(Default)]
pub struct State {
    pub capture_request_end:
        Option<sandproof_controller_service::capture::CaptureRequestEndObservation>,
    pub capture_request_end_observations: usize,
    pub terminal_drain_gate: Option<(u64, u64)>,
    pub terminal_drain_observations: usize,
    pub full_proposal_error: Option<ApplicationError>,
    pub source_probe: Option<SourceProbe>,
    pub source_calls: usize,
    pub actions: Vec<WorkAction<core::EffectId>>,
    pub configurations: BTreeMap<(u64, core::Generation), (String, CreateOptions)>,
    pub lose_activation: bool,
    pub block_prepare: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    pub block_release: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    pub block_health: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    pub released_slots: Vec<HostSlotContext>,
    pub bound_slots: Vec<HostSlotContext>,
    pub release_unknown: bool,
    pub disk_available: Option<u64>,
    pub disk_probe: Option<Box<dyn FnOnce() -> Result<u64, ApplicationError> + Send>>,
    pub template_error: Option<ApplicationError>,
    pub template_checks: usize,
    pub cached_verifications: usize,
    pub epoch: TestEpoch,
    pub epoch_checks: usize,
    pub recovery_observations: usize,
    pub source: Option<sandproof_controller_service::capture::SourceProcessIdentity>,
    pub observe_unavailable: bool,
    pub lose_stop: Option<core::SandboxId>,
    pub observe_hook: Option<Box<dyn FnOnce() + Send>>,
}
pub struct TestHost(pub Arc<Mutex<State>>);
impl WorkEvidence for TestHost {
    type Evidence = workflow::FakeHostEvidence;
}
impl WorkExecution for TestHost {
    fn propose_lifecycle_full(
        &self,
        _pause: &sandproof_controller_service::capture::PauseWorkRequest,
        _source: &sandproof_controller_service::capture::SourceObservationRequirement,
    ) -> Result<sandproof_controller_service::capture::FullCaptureWorkRequest, ApplicationError>
    {
        Err(self
            .0
            .lock()
            .unwrap()
            .full_proposal_error
            .unwrap_or(ApplicationError::Busy))
    }
    fn execute_lifecycle_original_continue(
        &self,
        action: &WorkAction<core::EffectId>,
        work: &sandproof_controller_service::capture::OriginalContinueWorkRequest,
        permit: &sandproof_controller_service::capture::OriginalContinuePermit,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError> {
        if permit.action() != action
            || permit.work_binding()
                != &work
                    .binding_digest()
                    .map_err(|_| WorkerTransportError::ConnectionLost)?
        {
            return Err(WorkerTransportError::ConnectionLost);
        }
        self.execute(action)
    }
    fn execute(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError> {
        let mut state = self.0.lock().unwrap();
        state.actions.push(action.clone());
        if matches!(action.command.command(), core::Command::PrepareVm { .. })
            && let Some((entered, release)) = state.block_prepare.take()
        {
            drop(state);
            entered.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(15))
                .unwrap();
            state = self.0.lock().unwrap();
        }
        if state.lose_activation
            && matches!(action.command.command(), core::Command::ActivateVm { .. })
        {
            return Err(WorkerTransportError::ReplyLost);
        }
        if matches!(action.command.command(), core::Command::StopVm { sandbox, .. }
            if state.lose_stop == Some(*sandbox))
        {
            return Err(WorkerTransportError::ReplyLost);
        }
        let mut receipt = workflow::worker(action)?;
        if let sandproof_protocol::WorkOutcome::ObservedSuccess(evidence) = &mut receipt.outcome {
            evidence.source.clone_from(&state.source);
        }
        Ok(receipt)
    }
}
impl WorkRecovery for TestHost {
    fn observe_lifecycle_capture_request_end(
        &self,
        _action: &WorkAction<core::EffectId>,
        _full: &sandproof_controller_service::capture::FullCaptureWorkRequest,
        _source: &sandproof_controller_service::capture::SourceObservationRequirement,
        _worker: &sandproof_controller_service::capture::CaptureWorkerBinding,
    ) -> Option<sandproof_controller_service::capture::CaptureRequestEndObservation> {
        let mut state = self.0.lock().unwrap();
        state.capture_request_end_observations += 1;
        state.capture_request_end.clone()
    }
    fn observe_lifecycle_terminated_capture(
        &self,
        work: &sandproof_controller_service::capture::TerminatedCaptureWorkRequest,
    ) -> Option<sandproof_controller_service::capture::TerminatedCaptureObservation> {
        let mut state = self.0.lock().unwrap();
        state.terminal_drain_observations += 1;
        let (device, inode) = state.terminal_drain_gate?;
        // Explicit controlled physical-boundary facts, never a real gate test.
        Some(sandproof_controller_service::capture::TerminatedCaptureObservation::StoppedAndDrained{
            work_binding:work.binding_digest().ok()?,full_work_binding:work.full().binding_digest().ok()?,
            seal_action:work.seal_action().clone(),stop_action:work.stop_action().clone(),
            source:work.source().source.clone(),worker:work.worker().cloned(),host_slot:work.host_slot(),
            helper_gate_device:device,helper_gate_inode:inode,
        })
    }
    fn observe_lifecycle_original_continue(
        &self,
        action: &WorkAction<core::EffectId>,
        _work: &sandproof_controller_service::capture::OriginalContinueWorkRequest,
    ) -> Option<WorkReceipt<Self::Evidence>> {
        self.observe(action)
    }
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>> {
        let mut state = self.0.lock().unwrap();
        state.recovery_observations += 1;
        if let Some(hook) = state.observe_hook.take() {
            hook();
        }
        (state.actions.contains(action) && !state.observe_unavailable).then(|| {
            let mut receipt = workflow::success(action);
            if let sandproof_protocol::WorkOutcome::ObservedSuccess(evidence) = &mut receipt.outcome
            {
                evidence.source.clone_from(&state.source);
            }
            receipt
        })
    }
    fn reclaim_incomplete_prepare(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }
    fn reclaim_incomplete_activation(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
    fn reclaim_incomplete_cleanup(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
}
impl HostSlots for TestHost {
    fn bind_host_slot(&self, context: &HostSlotContext, _: bool) -> Result<(), ApplicationError> {
        let mut state = self.0.lock().unwrap();
        state.bound_slots.push(*context);
        if state.released_slots.contains(context) {
            return Err(ApplicationError::Busy);
        }
        Ok(())
    }
    fn release_host_slot(&self, context: &HostSlotContext) -> Result<(), ApplicationError> {
        let blocked = self.0.lock().unwrap().block_release.take();
        if let Some((entered, release)) = blocked {
            entered.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(15))
                .unwrap();
        }
        let mut state = self.0.lock().unwrap();
        if state.release_unknown {
            Err(ApplicationError::Busy)
        } else {
            if !state.released_slots.contains(context) {
                state.released_slots.push(*context);
            }
            Ok(())
        }
    }
}
impl DataRelay for TestHost {
    fn acquire_data_gate(
        &self,
        _: core::SandboxId,
        _: core::VmId,
        _: core::Generation,
    ) -> Result<Option<std::fs::File>, ApplicationError> {
        let gate = tempfile::tempfile().map_err(|_| ApplicationError::Busy)?;
        gate.try_lock_shared().map_err(|_| ApplicationError::Busy)?;
        Ok(Some(gate))
    }
    fn data_gate_drained(&self, _: &HostSlotContext) -> Result<bool, ApplicationError> {
        Ok(true)
    }
}
impl HostCapacity for TestHost {
    fn observe_lifecycle_source(
        &self,
        requirement: &sandproof_controller_service::capture::SourceObservationRequirement,
    ) -> Result<sandproof_controller_service::capture::RawSourceObservation, WorkerTransportError>
    {
        let probe = {
            let mut state = self.0.lock().unwrap();
            state.source_calls += 1;
            state.source_probe.take()
        };
        probe.ok_or(WorkerTransportError::ConnectionLost)?(requirement)
    }
    fn observe_host_epoch(
        &self,
        requirement: &sandproof_controller_service::capture::HostEpochRequirement,
    ) -> Result<sandproof_controller_service::capture::HostEpochObservation, WorkerTransportError>
    {
        let mut state = self.0.lock().unwrap();
        state.epoch_checks += 1;
        if matches!(state.epoch, TestEpoch::Unavailable) {
            return Err(WorkerTransportError::ConnectionLost);
        }
        // Explicit component boundary fact only, never production attestation.
        let mut registration = requirement.registration().clone();
        if matches!(state.epoch, TestEpoch::WrongBoot) {
            registration.boot_id = "wrong-boot".into();
        }
        Ok(
            sandproof_controller_service::capture::HostEpochObservation {
                node: requirement.node().clone(),
                registration,
            },
        )
    }

    fn available_disk_bytes(&self) -> Result<u64, ApplicationError> {
        let (probe, available) = {
            let mut state = self.0.lock().unwrap();
            (
                state.disk_probe.take(),
                state.disk_available.unwrap_or(u64::MAX),
            )
        };
        probe.map_or(Ok(available), |probe| probe())
    }
    fn available_compute(&self) -> Result<(u64, u64), ApplicationError> {
        Ok((2, 512 * 1024 * 1024))
    }
    fn validate_physical_storage(
        &self,
        profile: &PhysicalStorageProfile,
        _: &PhysicalPool,
    ) -> Result<(), ApplicationError> {
        let blocked = self.0.lock().unwrap().block_health.take();
        if let Some((entered, release)) = blocked {
            entered.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(15))
                .unwrap();
        }
        profile.validate().map_err(|_| ApplicationError::Busy)
    }
}
impl LaunchConfiguration for TestHost {
    fn put_secret(
        &self,
        _: core::SandboxId,
        _: core::Generation,
        _: &str,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Invalid)
    }
    fn has_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> Result<bool, ApplicationError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .configurations
            .contains_key(&(sandbox.0, generation)))
    }
    fn token_for(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> Result<String, ApplicationError> {
        self.0
            .lock()
            .unwrap()
            .configurations
            .get(&(sandbox.0, generation))
            .map(|v| v.0.clone())
            .ok_or(ApplicationError::Busy)
    }
    fn put_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        token: &str,
        options: &CreateOptions,
    ) -> Result<(), ApplicationError> {
        let mut state = self.0.lock().unwrap();
        if state.configurations.contains_key(&(sandbox.0, generation)) {
            return Err(ApplicationError::Conflict);
        }
        state
            .configurations
            .insert((sandbox.0, generation), (token.into(), options.clone()));
        Ok(())
    }
    fn has_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        options: &CreateOptions,
    ) -> Result<bool, ApplicationError> {
        let state = self.0.lock().unwrap();
        match state.configurations.get(&(sandbox.0, generation)) {
            None => Ok(false),
            Some((_, original)) if original == options => Ok(true),
            _ => Err(ApplicationError::Conflict),
        }
    }
}
impl TemplateArtifacts for TestHost {
    fn cache_candidate_verification(
        &self,
        template: &TemplateBinding,
        verification: &CandidateVerification,
    ) {
        if verification.matches(&template.kernel) && verification.matches(&template.rootfs) {
            self.0.lock().unwrap().cached_verifications += 1;
        }
    }
    fn validate_template(&self, template: &TemplateBinding) -> Result<(), ApplicationError> {
        let mut state = self.0.lock().unwrap();
        state.template_checks += 1;
        if let Some(error) = state.template_error {
            return Err(error);
        }
        if template.is_valid() {
            Ok(())
        } else {
            Err(ApplicationError::Invalid)
        }
    }
}
pub fn template() -> InternalTemplate {
    let local = TemplateBinding {
        guest: None,
        template_id: "base-fixed".into(),
        revision: "r1".into(),
        owner: None,
        kernel: TemplateArtifact {
            path: "/protected-cache/kernel".into(),
            sha256: "a".repeat(64),
            size_bytes: 40,
        },
        rootfs: TemplateArtifact {
            path: "/protected-cache/rootfs".into(),
            sha256: "b".repeat(64),
            size_bytes: 4096,
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            host_memory_bytes: 129 * 1024 * 1024,
            disk_bytes: 4096,
        },
        envd_version: "fixed-envd".into(),
        startup: None,
    };
    InternalTemplate {
        revision: "template-revision".try_into().unwrap(),
        digest: local.canonical_digest().unwrap(),
        registered: local.clone(),
        local,
    }
}
pub fn compute() -> ComputeLimits {
    ComputeLimits {
        max_vcpus_global: 2,
        max_vcpus_per_tenant: 2,
        max_memory_bytes_global: 512 * 1024 * 1024,
        max_memory_bytes_per_tenant: 512 * 1024 * 1024,
    }
}
pub fn request() -> LifecycleRequest {
    let mut r = wire::request(1, 1, Intent::Create);
    r.create.timeout_ms = 60_000;
    r.create.max_lifetime_ms = 300_000;
    r.create.template_digest = template().digest;
    r.create.resources.memory_bytes = template().local.resources.host_memory_bytes;
    r.create.options = serde_json::to_vec(&CreateOptions {
        networking: Some(sandproof_controller_service::CreateNetworking {
            allow_internet_access: Some(false),
            network: None,
        }),
        ..CreateOptions::default()
    })
    .unwrap();
    r
}
pub fn application(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
) -> Arc<ControllerApplication<TestHost>> {
    application_with_inbox_config(
        path,
        state,
        clock,
        provision,
        InternalInboxConfig {
            node: wire::node("host-a"),
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
        },
    )
}

/// Selected component suites supply their explicit operation/options budget;
/// the original application and all local admission/recovery logic stay shared.
pub fn application_with_inbox_config(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
    inbox: InternalInboxConfig,
) -> Arc<ControllerApplication<TestHost>> {
    application_with_cache_config(path, state, clock, provision, inbox, None)
}
pub fn application_with_cache_config(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
    inbox: InternalInboxConfig,
    cache: Option<sandproof_controller_service::physical_storage::PhysicalTemplateCache>,
) -> Arc<ControllerApplication<TestHost>> {
    let reserve = workflow::CLEANUP_BYTES + 1000;
    let mut host = complete_fixture::host(path, workflow::NAMESPACE, 64, 2 * 1024 * 1024, reserve);
    if let Some(cache) = cache {
        let profile = host.physical_storage.as_mut().unwrap();
        profile.filesystem_overhead_inodes += cache.inode_budget;
        profile.host_template_caches = vec![cache];
    }
    application_with_host_config(path, state, clock, provision, inbox, host)
}

/// Real original `SQLite` authority with an explicitly selected registered pool.
pub fn application_with_host_config(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
    inbox: InternalInboxConfig,
    host: sandproof_controller_service::BoundedHostConfig,
) -> Arc<ControllerApplication<TestHost>> {
    let reserve = workflow::CLEANUP_BYTES + 1000;
    let mut journal = sandproof_controller_service::ControllerJournal::open_complete(
        path,
        workflow::NAMESPACE,
        host,
        complete_fixture::maintenance(),
        0,
    )
    .unwrap();
    if provision {
        journal
            .initialize_internal_inbox(inbox, &mut NeverCrash)
            .unwrap();
    }
    let mut limits = workflow::admission_limits();
    limits.cleanup_reserve_bytes = reserve;
    limits.max_disk_bytes_global = reserve + 2 * 1024 * 1024;
    Arc::new(
        ControllerApplication::new(
            journal,
            TestHost(state),
            ApplicationConfig {
                public_ipv4_enabled: false,
                limits,
                disk_bytes: 4096,
                file_policy_digest: core::PolicyDigest(1),
                cpu_count: 1,
                memory_mb: 128,
                disk_size_mb: 1,
                envd_version: "fixed-envd".into(),
                domain: "example.invalid".into(),
            },
        )
        .with_clock(clock),
    )
}
